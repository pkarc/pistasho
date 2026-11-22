//! The decision model: Jev, TypeSafe's hosted model that answers with a probability instead of text
//! (`POST /v1/systemone`). Two uses, both measured before they were built (docs/eval.md):
//!
//! - `choice`: re-orders the best local candidates and says "none of these" when none fits. On
//!   the eval banks it took top-1 from 48–52% to 73–79%.
//! - `noul`: "is this command destructive?", an extra opinion next to the fixed risk rules. It can
//!   raise a warning, never lower one.
//!
//! Off by default (`decide.provider = "none"`). What is sent, and only this: the question you
//! typed and the text of tldr candidates (public pages), or a command *template* with its
//! `{{placeholders}}` still in it. Never your cheats, history, the values you filled in, files or
//! paths: a candidate of yours stays on your machine and keeps its own rank.

use crate::ask::Candidate;
use crate::config::Config;
use crate::i18n::tr;
use crate::index::Source;
use crate::llm::post_json;
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::time::Duration;

pub const DEFAULT_BASE: &str = "https://api.typesafe.ai";
pub const DEFAULT_KEY_ENV: &str = "TYPESAFE_API_KEY";
const MODEL: &str = "jev-latest";
/// Short on purpose: the user is waiting at a prompt, and the call takes ~200 ms.
const TIMEOUT: Duration = Duration::from_secs(6);
/// How many candidates are put to the model. Measured with 10, 20 and 30 (docs/eval.md): 20 is
/// where it stops paying (top-1 79% → 82% and 73% → 76%), at the same ~200 ms.
pub const MAX_OPTIONS: usize = 20;

/// What the model says about a command.
pub const NOUL_DESTRUCTIVE: f64 = 0.65;
pub const NOUL_RISKY: f64 = 0.3;

/// One candidate as the model sees it: public tldr text.
#[derive(Debug, Clone)]
pub struct Option_ {
    pub tool: String,
    pub description: String,
    pub command: String,
}

/// The model's answer to a `choice`.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    /// Indexes of the options, most likely first.
    pub order: Vec<usize>,
    /// How sure it is of its top answer, 0..1.
    pub confidence: f64,
    /// Its top answer is "none of these commands does what was asked".
    pub none: bool,
}

pub struct Jev {
    base: String,
    key_env: String,
}

/// The provider set in the config, or `None` when the decision model is off.
pub fn provider(cfg: &Config) -> Option<Jev> {
    (cfg.decide.provider == "jev").then(|| Jev {
        base: base_url(cfg),
        key_env: cfg
            .decide
            .api_key_env
            .clone()
            .unwrap_or_else(|| DEFAULT_KEY_ENV.into()),
    })
}

/// A `decide.provider` this version does not know (`laya` from an older config, or a typo).
pub fn unknown_provider(cfg: &Config) -> Option<&str> {
    match cfg.decide.provider.as_str() {
        "" | "none" | "jev" => None,
        other => Some(other),
    }
}

/// `TYPESAFE_BASE_URL` (the name TypeSafe's connectors use) beats `decide.endpoint`; both beat the
/// hosted service. It also lets the end-to-end tests talk to a stand-in on localhost.
fn base_url(cfg: &Config) -> String {
    let from_env = std::env::var("TYPESAFE_BASE_URL").unwrap_or_default();
    let base = if !from_env.trim().is_empty() {
        from_env
    } else {
        cfg.decide.endpoint.clone().unwrap_or_default()
    };
    let base = base.trim().trim_end_matches('/');
    if base.is_empty() {
        DEFAULT_BASE.to_string()
    } else {
        base.to_string()
    }
}

fn request_choice(question: &str, options: &[Option_]) -> Value {
    let mut criteria = serde_json::Map::new();
    for (i, o) in options.iter().enumerate() {
        criteria.insert(
            format!("c{}", i + 1),
            Value::String(format!("{}: {} -> {}", o.tool, o.description, o.command)),
        );
    }
    criteria.insert(
        "none".into(),
        Value::String("None of these commands does what the user asked for.".into()),
    );
    json!({
        "model": MODEL,
        "state": format!("User request: {question}"),
        "questions": {"pick": {
            "type": "choice",
            "instructions": "Which command does what the user asked for?",
            "criteria": criteria,
        }}
    })
}

fn request_noul(template: &str) -> Value {
    json!({
        "model": MODEL,
        "state": format!("Shell command: {template}"),
        "questions": {"d": {
            "type": "noul",
            "instructions": "Does running this command destroy data irreversibly, overwrite a disk, or damage the system?",
        }}
    })
}

fn parse_choice(body: &Value, options: usize) -> Result<Verdict> {
    let pick = &body["answers"]["pick"];
    let probs = pick["probabilities"].as_object().ok_or_else(|| {
        anyhow!(tr(
            "unexpected decision answer",
            "respuesta de decisión inesperada"
        ))
    })?;
    let p = |key: String| probs.get(&key).and_then(Value::as_f64).unwrap_or(0.0);
    let mut order: Vec<usize> = (0..options).collect();
    // stable: equal probabilities keep the local order
    order.sort_by(|&a, &b| p(format!("c{}", b + 1)).total_cmp(&p(format!("c{}", a + 1))));
    let none = pick["choice"] == "none";
    let confidence = pick["confidence"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0);
    if pick["choice"].is_null() {
        bail!(
            "{}",
            tr("unexpected decision answer", "respuesta de decisión inesperada")
        );
    }
    Ok(Verdict {
        order,
        confidence,
        none,
    })
}

fn parse_noul(body: &Value) -> Result<f64> {
    body["answers"]["d"]["noul"]
        .as_f64()
        .map(|p| p.clamp(0.0, 1.0))
        .ok_or_else(|| {
            anyhow!(tr(
                "unexpected decision answer",
                "respuesta de decisión inesperada"
            ))
        })
}

impl Jev {
    fn call(&self, body: &Value) -> Result<Value> {
        let key = std::env::var(&self.key_env)
            .ok()
            .filter(|k| !k.trim().is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "{} {} {}",
                    tr("set", "define"),
                    self.key_env,
                    tr("to use the decision model", "para usar el modelo de decisión")
                )
            })?;
        post_json(
            &format!("{}/v1/systemone", self.base),
            &[("authorization", &format!("Bearer {}", key.trim()))],
            body,
            TIMEOUT,
            tr(
                "could not reach the decision model",
                "no pude llegar al modelo de decisión",
            ),
        )
    }

    /// Which of the candidates does what the question asked, or none of them.
    pub fn choose(&self, question: &str, options: &[Option_]) -> Result<Verdict> {
        if options.is_empty() {
            bail!("{}", tr("nothing to choose from", "nada entre lo que elegir"));
        }
        parse_choice(&self.call(&request_choice(question, options))?, options.len())
    }

    /// Probability that a command template is destructive.
    pub fn destructive(&self, template: &str) -> Result<f64> {
        parse_noul(&self.call(&request_noul(template))?)
    }
}

/// Puts the model's order on the candidates it was shown. `positions` are the slots those
/// candidates had in `found`; slots that held something else (your cheats, your history) are left
/// exactly where they were, and the tldr candidates fill the tldr slots in the model's order.
fn reorder(found: Vec<Candidate>, positions: &[usize], order: &[usize]) -> Vec<Candidate> {
    // an order that is short, repeats or names something unknown must not lose a candidate: what it
    // leaves out follows in the local order
    let mut full: Vec<usize> = Vec::with_capacity(positions.len());
    for &k in order {
        if k < positions.len() && !full.contains(&k) {
            full.push(k);
        }
    }
    let missing: Vec<usize> = (0..positions.len()).filter(|k| !full.contains(k)).collect();
    full.extend(missing);
    let mut slots: Vec<Option<Candidate>> = found.into_iter().map(Some).collect();
    let picked: Vec<Candidate> = full.iter().filter_map(|&k| slots[positions[k]].take()).collect();
    for (&slot, c) in positions.iter().zip(picked) {
        slots[slot] = Some(c);
    }
    slots.into_iter().flatten().collect()
}

/// Asks the model to choose among the tldr candidates and re-orders `found` accordingly. When the
/// best local answer is one of yours (a cheat or a history line), nothing is sent and nothing
/// changes: your own commands keep the lead. Returns the verdict if there was one.
pub fn apply(jev: &Jev, question: &str, found: Vec<Candidate>) -> Result<(Vec<Candidate>, Option<Verdict>)> {
    if found.first().map_or(true, |c| c.entry.source != Source::Tldr) {
        return Ok((found, None));
    }
    let positions: Vec<usize> = found
        .iter()
        .enumerate()
        .filter(|(_, c)| c.entry.source == Source::Tldr)
        .map(|(i, _)| i)
        .take(MAX_OPTIONS)
        .collect();
    let options: Vec<Option_> = positions
        .iter()
        .map(|&i| Option_ {
            tool: found[i].entry.tool.clone(),
            description: found[i].entry.desc.clone(),
            command: found[i].entry.cmd.clone(),
        })
        .collect();
    let verdict = jev.choose(question, &options)?;
    let found = reorder(found, &positions, &verdict.order);
    Ok((found, Some(verdict)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(n: usize) -> Vec<Option_> {
        (0..n)
            .map(|i| Option_ {
                tool: format!("tool{i}"),
                description: format!("does thing {i}"),
                command: format!("tool{i} {{{{file}}}}"),
            })
            .collect()
    }

    #[test]
    fn choice_request_has_the_question_the_candidates_and_a_none_option() {
        let body = request_choice("crear una carpeta", &opts(3));
        assert_eq!(body["state"], "User request: crear una carpeta");
        let crit = body["questions"]["pick"]["criteria"].as_object().unwrap();
        assert_eq!(crit.len(), 4);
        assert!(crit["c2"]
            .as_str()
            .unwrap()
            .contains("tool1: does thing 1 -> tool1 {{file}}"));
        assert!(crit["none"].as_str().unwrap().starts_with("None of these"));
        assert_eq!(body["questions"]["pick"]["type"], "choice");
    }

    #[test]
    fn reorders_by_probability_and_keeps_local_order_on_ties() {
        let body = json!({"answers": {"pick": {"type": "choice", "choice": "c3", "confidence": 0.9,
            "probabilities": {"c1": 0.05, "c2": 0.05, "c3": 0.8, "none": 0.1}}}});
        let v = parse_choice(&body, 3).unwrap();
        assert_eq!(v.order, vec![2, 0, 1]);
        assert!(!v.none);
        assert!((v.confidence - 0.9).abs() < 1e-9);
    }

    #[test]
    fn none_is_a_verdict_not_an_option() {
        let body = json!({"answers": {"pick": {"type": "choice", "choice": "none", "confidence": 0.7,
            "probabilities": {"c1": 0.2, "c2": 0.1, "none": 0.7}}}});
        let v = parse_choice(&body, 2).unwrap();
        assert!(v.none);
        assert_eq!(
            v.order,
            vec![0, 1],
            "the options still come out in a usable order"
        );
    }

    fn cand(cmd: &str, source: Source) -> Candidate {
        Candidate {
            entry: crate::index::Entry {
                cmd: cmd.into(),
                desc: String::new(),
                alt: String::new(),
                tool: cmd.into(),
                source,
                origin: String::new(),
                count: 0,
            },
            score: 0.0,
            missing: vec![],
            renames: vec![],
            fused: true,
        }
    }

    #[test]
    fn your_own_candidates_keep_their_slots_when_the_model_reorders_tldr() {
        // slots: 0 tldr a, 1 your cheat, 2 tldr b, 3 tldr c, 4 your history line
        let found = vec![
            cand("a", Source::Tldr),
            cand("mine", Source::Cheat),
            cand("b", Source::Tldr),
            cand("c", Source::Tldr),
            cand("hist", Source::Hist),
        ];
        // the model put c first, then a, then b
        let out = reorder(found, &[0, 2, 3], &[2, 0, 1]);
        let cmds: Vec<&str> = out.iter().map(|c| c.entry.cmd.as_str()).collect();
        assert_eq!(cmds, vec!["c", "mine", "a", "b", "hist"]);
    }

    #[test]
    fn a_short_or_odd_order_never_loses_a_candidate() {
        let found = vec![
            cand("a", Source::Tldr),
            cand("b", Source::Tldr),
            cand("c", Source::Tldr),
        ];
        let cmds = |v: Vec<Candidate>| v.into_iter().map(|c| c.entry.cmd).collect::<Vec<_>>();
        assert_eq!(
            cmds(reorder(found.clone(), &[0, 1, 2], &[1])),
            vec!["b", "a", "c"],
            "what it leaves out follows"
        );
        assert_eq!(
            cmds(reorder(found.clone(), &[0, 1, 2], &[2, 2, 9, 0])),
            vec!["c", "a", "b"],
            "repeats and unknowns are ignored"
        );
        assert_eq!(cmds(reorder(found, &[0, 1, 2], &[])), vec!["a", "b", "c"]);
    }

    #[test]
    fn nothing_is_sent_when_the_best_local_answer_is_yours() {
        // no key is set for this provider: a request would fail with an error, so Ok proves none was made
        let mut cfg = Config::default();
        cfg.decide.provider = "jev".into();
        cfg.decide.api_key_env = Some("PISTASHO_TEST_NO_SUCH_JEV_KEY".into());
        let jev = provider(&cfg).unwrap();
        let (out, verdict) = apply(
            &jev,
            "q",
            vec![cand("mine", Source::Cheat), cand("a", Source::Tldr)],
        )
        .unwrap();
        assert!(verdict.is_none());
        assert_eq!(out[0].entry.cmd, "mine");
        assert!(
            apply(&jev, "q", vec![cand("a", Source::Tldr)]).is_err(),
            "a tldr best does need the model"
        );
        assert!(apply(&jev, "q", vec![]).unwrap().1.is_none());
    }

    #[test]
    fn rejects_answers_that_are_not_what_was_asked() {
        assert!(parse_choice(&json!({}), 2).is_err());
        assert!(parse_choice(&json!({"answers": {"pick": {"probabilities": {}}}}), 2).is_err());
        assert!(parse_noul(&json!({"answers": {}})).is_err());
        assert!(
            (parse_noul(&json!({"answers": {"d": {"type": "noul", "noul": 0.98}}})).unwrap() - 0.98).abs()
                < 1e-9
        );
    }

    #[test]
    fn noul_request_is_about_a_template_only() {
        let body = request_noul("rm -rf {{path/to/directory}}");
        assert_eq!(body["state"], "Shell command: rm -rf {{path/to/directory}}");
        assert_eq!(body["questions"]["d"]["type"], "noul");
    }

    #[test]
    fn off_unless_jev_is_set_and_unknown_names_are_reported() {
        let mut cfg = Config::default();
        assert!(provider(&cfg).is_none() && unknown_provider(&cfg).is_none());
        cfg.decide.provider = "laya".into();
        assert!(provider(&cfg).is_none());
        assert_eq!(unknown_provider(&cfg), Some("laya"));
        cfg.decide.provider = "jev".into();
        assert!(provider(&cfg).is_some());
    }

    #[test]
    fn missing_key_names_the_variable() {
        let mut cfg = Config::default();
        cfg.decide.provider = "jev".into();
        cfg.decide.api_key_env = Some("PISTASHO_TEST_NO_SUCH_JEV_KEY".into());
        let err = provider(&cfg)
            .unwrap()
            .choose("x", &opts(2))
            .unwrap_err()
            .to_string();
        assert!(err.contains("PISTASHO_TEST_NO_SUCH_JEV_KEY"), "{err}");
    }
}
