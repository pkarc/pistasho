//! The LLM fallback: when the local index has nothing for a question, ask a model for one shell
//! command. Off by default (`llm.provider = "none"`): nothing leaves the machine unless you set a
//! provider.
//!
//! What is sent, and only this: the question you typed, the OS name, the package manager and the
//! shell name. Never your history, files, paths or environment.
//!
//! The answer is a suggestion like any other: it goes through the same binary check and risk rules
//! before it reaches the prompt, and it is never run.

use crate::config::Config;
use crate::i18n::tr;
use crate::packages;
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Haiku 4.5: fast and cheap, which is what a one-line command needs.
pub const DEFAULT_MODEL: &str = "claude-haiku-4-5";
pub const DEFAULT_KEY_ENV: &str = "ANTHROPIC_API_KEY";
/// A command is one line: a long answer is a mistake, not a better answer.
const MAX_TOKENS: u32 = 1024;
/// Short on purpose: the user is waiting at a prompt.
const TIMEOUT: Duration = Duration::from_secs(20);

/// What the model is told about the machine. Nothing else about the user is ever included.
#[derive(Debug, Clone)]
pub struct Machine {
    pub os: String,
    pub package_manager: String,
    pub shell: String,
}

impl Machine {
    pub fn detect() -> Self {
        let os = packages::detect_os().map_or_else(
            || "Linux".to_string(),
            |o| {
                if o.pretty_name.is_empty() {
                    o.id
                } else {
                    o.pretty_name
                }
            },
        );
        let package_manager = packages::Manager::detect()
            .map_or("none", |m| m.name())
            .to_string();
        let shell = std::env::var("SHELL")
            .ok()
            .and_then(|s| s.rsplit('/').next().map(str::to_string))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "sh".into());
        Self {
            os,
            package_manager,
            shell,
        }
    }
}

/// What a model suggests.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Suggestion {
    pub command: String,
    #[serde(default)]
    pub binaries: Vec<String>,
    /// Package that provides a program not installed by default; empty when none is needed.
    #[serde(default)]
    pub package: String,
    /// `ok`, `caution` or `destructive`. Advisory: our own rules decide, and this can only add a
    /// warning.
    #[serde(default)]
    pub risk: String,
    #[serde(default)]
    pub explanation: String,
}

pub struct Answer {
    pub suggestion: Suggestion,
    pub model: String,
    pub took: Duration,
    /// Came from the cache of an earlier identical question: no request was made.
    pub cached: bool,
}

/// Something that can turn a question into a command. One implementation per provider.
pub trait Generate {
    fn model(&self) -> &str;
    fn generate(&self, question: &str, machine: &Machine) -> Result<Suggestion>;
}

/// The provider set in the config, or `None` when the LLM is off.
pub fn provider(cfg: &Config) -> Option<Box<dyn Generate>> {
    match cfg.llm.provider.as_str() {
        "anthropic" => Some(Box::new(Anthropic {
            model: cfg.llm.model.clone().unwrap_or_else(|| DEFAULT_MODEL.into()),
            key_env: cfg
                .llm
                .api_key_env
                .clone()
                .unwrap_or_else(|| DEFAULT_KEY_ENV.into()),
        })),
        _ => None,
    }
}

/// A provider name in the config that this version does not know (`ollama` from an older config,
/// or a typo): said out loud instead of silently behaving as if the LLM were off.
pub fn unknown_provider(cfg: &Config) -> Option<&str> {
    match cfg.llm.provider.as_str() {
        "" | "none" | "anthropic" => None,
        other => Some(other),
    }
}

/// Best local score under which the LLM is asked (`llm.threshold` overrides it). Chosen from
/// `pistasho eval --confidence`, see docs/eval.md.
pub const DEFAULT_THRESHOLD: f64 = 0.20;

/// Whether to ask the LLM for this question, given the best local candidate: a provider is set
/// and either nothing matched or (with `when = "unsure"`) the match is shaky.
///
/// `jev_none` is the decision model's verdict when it looked at the candidates: whether it found
/// none of them right. It knows better than the score does, so when there is one it decides.
pub fn wanted(cfg: &Config, best: Option<&crate::ask::Candidate>, jev_none: Option<bool>) -> bool {
    if cfg.llm.provider.is_empty() || cfg.llm.provider == "none" {
        return false;
    }
    match (cfg.llm.when.as_str(), jev_none) {
        ("nomatch", _) => best.is_none(),
        (_, Some(none)) => none || best.is_none(),
        _ => crate::ask::is_unsure(best, cfg.llm.threshold.unwrap_or(DEFAULT_THRESHOLD)),
    }
}

/// The LLM's answer to a question: from the cache if the same question (in any form of
/// capitalisation, accents or punctuation) was asked before, otherwise from the provider, and then
/// remembered. `Ok(None)` means the LLM is off. A repeated question costs nothing.
pub fn resolve(cfg: &Config, db: &crate::db::Db, question: &str) -> Result<Option<Answer>> {
    let Some(provider) = provider(cfg) else {
        return Ok(None);
    };
    let key = crate::ask::normalize_question(question);
    if let Ok(Some(hit)) = db.cache_get(&key) {
        return Ok(Some(Answer {
            suggestion: Suggestion {
                command: hit.command,
                binaries: hit.binaries,
                package: hit.package,
                risk: hit.risk,
                explanation: hit.explanation,
            },
            model: hit.model,
            took: Duration::ZERO,
            cached: true,
        }));
    }
    let started = Instant::now();
    let suggestion = provider.generate(question, &Machine::detect())?;
    // a failed write only means it is asked again next time
    let _ = db.cache_put(&crate::db::CachedAnswer {
        question: key,
        command: suggestion.command.clone(),
        binaries: suggestion.binaries.clone(),
        package: suggestion.package.clone(),
        risk: suggestion.risk.clone(),
        explanation: suggestion.explanation.clone(),
        model: provider.model().to_string(),
    });
    Ok(Some(Answer {
        suggestion,
        model: provider.model().to_string(),
        took: started.elapsed(),
        cached: false,
    }))
}

/// The line under the prompt for an answer from a model: `✦ llm · claude-haiku-4-5 · 1.1 s`, and a
/// warning if the model itself thinks the command is risky.
pub fn message(answer: &Answer) -> String {
    let mut message = if answer.cached {
        format!("⚡ local · {}", tr("cache", "caché"))
    } else {
        format!("✦ llm · {} · {:.1} s", answer.model, answer.took.as_secs_f64())
    };
    match answer.suggestion.risk.as_str() {
        "" | "ok" => {}
        risk => message.push_str(&format!(" · ⚠ llm: {risk}")),
    }
    message
}

// ---------------------------------------------------------------- Anthropic

pub struct Anthropic {
    pub model: String,
    /// Name of the environment variable that holds the API key.
    pub key_env: String,
}

/// `ANTHROPIC_BASE_URL` (the variable the Anthropic SDKs honour) points it at a proxy, or at a
/// local stand-in in the end-to-end tests.
fn messages_url() -> String {
    let base = std::env::var("ANTHROPIC_BASE_URL").unwrap_or_default();
    let base = base.trim().trim_end_matches('/');
    format!(
        "{}/v1/messages",
        if base.is_empty() { ANTHROPIC_BASE_URL } else { base }
    )
}

fn system_prompt(machine: &Machine) -> String {
    format!(
        "You turn a request into ONE shell command for the user's machine.\n\
         Machine: {os}; package manager: {pm}; shell: {shell}.\n\
         Rules:\n\
         - Answer only with the JSON object of the schema.\n\
         - `command` is a single line (pipes and && are fine), ready to type at a prompt. Never use \
           placeholders such as {{{{name}}}}: use realistic concrete values (a file name, a port) the user can edit.\n\
         - Prefer tools present on a default install. If the command needs a program that usually is \
           not installed, list every program the command runs in `binaries` and the package that \
           provides the missing one in `package`; otherwise leave `package` empty.\n\
         - If the request is ambiguous, pick the most likely meaning and say so in `explanation`.\n\
         - `risk` is `destructive` if the command deletes or overwrites data or changes the system, \
           `caution` if it changes files in place, otherwise `ok`.\n\
         - `explanation` is one short sentence, in the language of the request.\n\
         - The command is only shown to the user, who decides whether to run it. Do not claim to run anything.",
        os = machine.os,
        pm = machine.package_manager,
        shell = machine.shell,
    )
}

/// JSON schema of the answer (structured outputs: every field required, no extras).
fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "command": {"type": "string"},
            "binaries": {"type": "array", "items": {"type": "string"}},
            "package": {"type": "string"},
            "risk": {"type": "string", "enum": ["ok", "caution", "destructive"]},
            "explanation": {"type": "string"}
        },
        "required": ["command", "binaries", "package", "risk", "explanation"],
        "additionalProperties": false
    })
}

impl Anthropic {
    /// The request body. Structured outputs rather than a forced tool call: newer models reject
    /// `tool_choice` of `any` / `tool`, and this works on all of them.
    fn request_body(&self, question: &str, machine: &Machine) -> Value {
        json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "system": system_prompt(machine),
            "messages": [{"role": "user", "content": question}],
            "output_config": {"format": {"type": "json_schema", "schema": schema()}}
        })
    }
}

impl Generate for Anthropic {
    fn model(&self) -> &str {
        &self.model
    }

    fn generate(&self, question: &str, machine: &Machine) -> Result<Suggestion> {
        let key = std::env::var(&self.key_env)
            .ok()
            .filter(|k| !k.trim().is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "{} {} {}",
                    tr("set", "define"),
                    self.key_env,
                    tr("to use the LLM", "para usar el LLM")
                )
            })?;
        let body = post_json(
            &messages_url(),
            &[
                ("x-api-key", key.trim()),
                ("anthropic-version", ANTHROPIC_VERSION),
            ],
            &self.request_body(question, machine),
            TIMEOUT,
            tr("could not reach the LLM", "no pude llegar al LLM"),
        )?;
        parse_response(&body)
    }
}

/// POSTs JSON and returns the JSON answer. Errors say why in a short line and never echo the
/// request (which holds the question and, in a header, the key).
pub fn post_json(
    url: &str,
    headers: &[(&str, &str)],
    body: &Value,
    timeout: Duration,
    unreachable: &str,
) -> Result<Value> {
    let mut request = ureq::post(url)
        .timeout(timeout)
        .set("content-type", "application/json");
    for (name, value) in headers {
        request = request.set(name, value);
    }
    match request.send_string(&body.to_string()) {
        Ok(r) => {
            let text = r
                .into_string()
                .context(tr("unreadable LLM response", "respuesta del LLM ilegible"))?;
            serde_json::from_str(&text).context(tr("unreadable LLM response", "respuesta del LLM ilegible"))
        }
        Err(ureq::Error::Status(code, r)) => {
            let value = r
                .into_string()
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok());
            // {"error": {"message": ..}}, or a plain {"error": ".."}
            let detail = value
                .as_ref()
                .and_then(|v| v["error"]["message"].as_str().or_else(|| v["error"].as_str()))
                .unwrap_or_default()
                .to_string();
            bail!("{}", status_message(code, &detail))
        }
        Err(e) => bail!("{unreachable} ({e})"),
    }
}

/// A short, readable reason for an HTTP error, without ever echoing the request.
fn status_message(code: u16, detail: &str) -> String {
    let what = match code {
        401 | 403 => tr("the API key was rejected", "la API key fue rechazada"),
        404 => tr("unknown model", "modelo desconocido"),
        429 => tr(
            "rate limited, try again in a moment",
            "límite de uso, prueba en un momento",
        ),
        529 | 503 => tr("the service is overloaded", "el servicio está saturado"),
        _ => tr("the service returned an error", "el servicio devolvió un error"),
    };
    if detail.is_empty() {
        format!("{what} (HTTP {code})")
    } else {
        format!("{what} (HTTP {code}: {detail})")
    }
}

/// Reads the suggestion out of a Messages API response.
fn parse_response(body: &Value) -> Result<Suggestion> {
    if body["stop_reason"] == "refusal" {
        bail!(
            "{}",
            tr("the LLM declined to answer", "el LLM no quiso responder")
        );
    }
    let text = body["content"]
        .as_array()
        .and_then(|blocks| blocks.iter().find(|b| b["type"] == "text"))
        .and_then(|b| b["text"].as_str())
        .ok_or_else(|| anyhow!(tr("empty LLM response", "respuesta del LLM vacía")))?;
    if body["stop_reason"] == "max_tokens" {
        bail!(
            "{}",
            tr("the LLM answer was cut off", "la respuesta del LLM se cortó")
        );
    }
    parse_suggestion(text)
}

/// The JSON the model wrote, checked: one non-empty line of command.
fn parse_suggestion(text: &str) -> Result<Suggestion> {
    let mut suggestion: Suggestion = serde_json::from_str(text).context(tr(
        "the LLM did not answer in the expected format",
        "el LLM no respondió en el formato esperado",
    ))?;
    suggestion.command = suggestion.command.trim().to_string();
    if suggestion.command.is_empty() {
        bail!(
            "{}",
            tr("the LLM gave no command", "el LLM no dio ningún comando")
        );
    }
    // a prompt takes one line: a multi-line answer would run as several commands when pasted
    if suggestion.command.contains('\n') {
        bail!(
            "{}",
            tr("the LLM gave more than one line", "el LLM dio más de una línea")
        );
    }
    Ok(suggestion)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine() -> Machine {
        Machine {
            os: "Ubuntu 24.04".into(),
            package_manager: "apt".into(),
            shell: "zsh".into(),
        }
    }

    fn answer(text: &str) -> Value {
        json!({"stop_reason": "end_turn", "content": [{"type": "text", "text": text}]})
    }

    #[test]
    fn request_carries_only_the_question_and_the_machine() {
        let a = Anthropic {
            model: "claude-haiku-4-5".into(),
            key_env: "X".into(),
        };
        let body = a.request_body("renombrar .jpeg a .jpg", &machine());
        assert_eq!(body["model"], "claude-haiku-4-5");
        assert_eq!(body["messages"][0]["content"], "renombrar .jpeg a .jpg");
        let system = body["system"].as_str().unwrap();
        assert!(system.contains("Ubuntu 24.04") && system.contains("apt") && system.contains("zsh"));
        // structured outputs, not a forced tool call
        assert_eq!(body["output_config"]["format"]["type"], "json_schema");
        assert!(body.get("tool_choice").is_none() && body.get("tools").is_none());
        // the whole body is the question, the machine and the fixed instructions
        let text = body.to_string();
        assert!(!text.contains("HISTFILE") && !text.contains("/home/"));
    }

    #[test]
    fn schema_requires_every_field_and_allows_no_extras() {
        let s = schema();
        assert_eq!(s["additionalProperties"], false);
        let required: Vec<&str> = s["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        for k in s["properties"].as_object().unwrap().keys() {
            assert!(required.contains(&k.as_str()), "{k} must be required");
        }
    }

    #[test]
    fn parses_a_good_answer() {
        let s = parse_response(&answer(
            r#"{"command":"for f in *.jpeg; do mv \"$f\" \"${f%.jpeg}.jpg\"; done","binaries":["mv"],"package":"","risk":"caution","explanation":"Renombra."}"#,
        ))
        .unwrap();
        assert_eq!(s.command, r#"for f in *.jpeg; do mv "$f" "${f%.jpeg}.jpg"; done"#);
        assert_eq!(s.binaries, vec!["mv"]);
        assert_eq!(s.risk, "caution");
    }

    #[test]
    fn rejects_answers_that_are_not_one_usable_command() {
        for bad in [
            r#"{"command":"","binaries":[],"package":"","risk":"ok","explanation":""}"#,
            r#"{"command":"ls\nrm -rf x","binaries":[],"package":"","risk":"ok","explanation":""}"#,
            "not json at all",
            r#"{"binaries":[]}"#,
        ] {
            assert!(parse_response(&answer(bad)).is_err(), "{bad}");
        }
        assert!(parse_response(&json!({"stop_reason": "refusal", "content": []})).is_err());
        assert!(parse_response(
            &json!({"stop_reason": "max_tokens", "content": [{"type": "text", "text": "{\"command\":\"ls"}]})
        )
        .is_err());
        assert!(parse_response(&json!({"content": []})).is_err());
    }

    #[test]
    fn message_names_the_model_and_passes_on_the_models_own_warning() {
        let mut a = Answer {
            suggestion: Suggestion {
                command: "ls".into(),
                binaries: vec![],
                package: String::new(),
                risk: "ok".into(),
                explanation: String::new(),
            },
            model: "claude-haiku-4-5".into(),
            took: Duration::from_millis(1100),
            cached: false,
        };
        assert_eq!(message(&a), "✦ llm · claude-haiku-4-5 · 1.1 s");
        a.suggestion.risk = "destructive".into();
        assert!(message(&a).ends_with("⚠ llm: destructive"));
        a.cached = true;
        assert!(message(&a).starts_with("⚡ local · cache"));
    }

    #[test]
    fn unsure_asks_on_a_shaky_fused_match_and_nomatch_only_when_nothing_matches() {
        let mut cfg = Config::default();
        cfg.llm.provider = "anthropic".into();
        let mut best = crate::ask::Candidate {
            entry: crate::index::Entry {
                cmd: "ls".into(),
                desc: String::new(),
                alt: String::new(),
                tool: "ls".into(),
                source: crate::index::Source::Tldr,
                origin: String::new(),
                count: 0,
            },
            score: 0.12,
            missing: vec![],
            renames: vec![],
            fused: true,
        };
        // default: `unsure`, threshold 0.20
        assert!(wanted(&cfg, None, None));
        assert!(wanted(&cfg, Some(&best), None), "0.12 is shaky");
        best.score = 0.25;
        assert!(!wanted(&cfg, Some(&best), None), "0.25 is solid");
        cfg.llm.threshold = Some(0.30);
        assert!(wanted(&cfg, Some(&best), None), "the threshold is configurable");
        // the decision model's verdict beats the score either way
        assert!(!wanted(&cfg, Some(&best), Some(false)));
        assert!(wanted(&cfg, Some(&best), Some(true)));
        cfg.llm.when = "nomatch".into();
        assert!(!wanted(&cfg, Some(&best), None));
        assert!(
            !wanted(&cfg, Some(&best), Some(true)),
            "nomatch only asks when nothing matched"
        );
        assert!(wanted(&cfg, None, None));
    }

    #[test]
    fn a_repeated_question_comes_from_the_cache_and_costs_no_request() {
        let mut cfg = Config::default();
        cfg.llm.provider = "anthropic".into();
        // a key that does not exist: any attempt to reach the provider would fail
        cfg.llm.api_key_env = Some("PISTASHO_TEST_NO_SUCH_KEY".into());
        let db = crate::db::Db::open_in_memory().unwrap();
        db.cache_put(&crate::db::CachedAnswer {
            question: crate::ask::normalize_question("¿Cómo renombro los .jpeg?"),
            command: "rename s/jpeg/jpg/ *.jpeg".into(),
            binaries: vec!["rename".into()],
            package: String::new(),
            risk: "caution".into(),
            explanation: String::new(),
            model: "m".into(),
        })
        .unwrap();
        let hit = resolve(&cfg, &db, "como RENOMBRO los jpeg").unwrap().unwrap();
        assert!(hit.cached);
        assert_eq!(hit.suggestion.command, "rename s/jpeg/jpg/ *.jpeg");
        assert_eq!(hit.suggestion.binaries, vec!["rename"]);
        // another question has no cache and needs the provider (here: fails for lack of a key)
        assert!(resolve(&cfg, &db, "otra cosa distinta").is_err());
    }

    #[test]
    fn an_unknown_provider_is_named_not_ignored() {
        let mut cfg = Config::default();
        assert_eq!(unknown_provider(&cfg), None);
        cfg.llm.provider = "ollama".into();
        assert_eq!(unknown_provider(&cfg), Some("ollama"));
        assert!(provider(&cfg).is_none());
    }

    #[test]
    fn llm_is_off_unless_a_provider_is_set() {
        let mut cfg = Config::default();
        assert!(provider(&cfg).is_none());
        let db = crate::db::Db::open_in_memory().unwrap();
        assert!(resolve(&cfg, &db, "anything").unwrap().is_none());
        assert!(!wanted(&cfg, None, Some(true)), "off means never asked");
        cfg.llm.provider = "anthropic".into();
        assert_eq!(provider(&cfg).unwrap().model(), DEFAULT_MODEL);
    }

    #[test]
    fn missing_key_says_which_variable_to_set() {
        let a = Anthropic {
            model: DEFAULT_MODEL.into(),
            key_env: "PISTASHO_TEST_NO_SUCH_KEY".into(),
        };
        let err = a.generate("x", &machine()).unwrap_err().to_string();
        assert!(err.contains("PISTASHO_TEST_NO_SUCH_KEY"), "{err}");
    }

    #[test]
    fn http_errors_have_short_readable_reasons() {
        assert!(status_message(401, "").contains("HTTP 401"));
        assert!(status_message(429, "slow down").contains("slow down"));
    }
}
