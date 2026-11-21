//! The embedding model: `multilingual-e5-small` (int8) on ONNX Runtime. Chosen in task 2.9a, see
//! docs/eval.md. It turns a question or an entry into a 384-number vector so that "montar una
//! partición" can find `mount` even though no word in common.
//!
//! Loading the model costs ~0.4–0.5 s (the tokenizer is the slow half), so `prefetch` starts it
//! on a thread while the user is still typing the question, and everything here fails soft: no
//! model, no vectors or an error means plain BM25, never a failed `ask`.

use crate::config::{self, Config};
use crate::i18n::tr;
use crate::semantic::{self, DIM};
use anyhow::{anyhow, bail, Context, Result};
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread::JoinHandle;

/// Bump when the model or the way text is turned into vectors changes: stored vectors are
/// thrown away and rebuilt.
pub const MODEL_ID: &str = "e5-small-q8-1";

/// Pinned to a commit, with the hash of each file: what is downloaded is what was measured.
const REVISION: &str = "761b726dd34fb83930e26aab4e9ac3899aa1fa78";
const FILES: &[(&str, &str, &str)] = &[
    (
        "onnx/model_quantized.onnx",
        "model.onnx",
        "f80102d3f2a1229f387d3c81909990d8945513e347b0eab049f7de3c6f98c193",
    ),
    (
        "tokenizer.json",
        "tokenizer.json",
        "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
    ),
];

/// Longer passages are cut: the start says what a command is for.
const MAX_TOKENS: usize = 128;
const BATCH: usize = 32;

pub fn model_dir() -> Result<PathBuf> {
    Ok(config::data_dir()?.join("models").join("multilingual-e5-small"))
}

pub fn model_present() -> bool {
    model_dir().is_ok_and(|d| FILES.iter().all(|(_, name, _)| d.join(name).is_file()))
}

/// Downloads the model if it is not there yet, checking every file against its pinned hash.
pub fn ensure_model() -> Result<()> {
    if model_present() {
        return Ok(());
    }
    let dir = model_dir()?;
    std::fs::create_dir_all(&dir)?;
    for (remote, name, sha) in FILES {
        let target = dir.join(name);
        if target.is_file() {
            continue;
        }
        let url = format!("https://huggingface.co/Xenova/multilingual-e5-small/resolve/{REVISION}/{remote}");
        eprintln!("{} {name} …", tr("Downloading model", "Descargando modelo"));
        let resp = ureq::get(&url)
            .call()
            .with_context(|| format!("{} {url}", tr("download failed:", "falló la descarga:")))?;
        let mut bytes = Vec::new();
        resp.into_reader()
            .take(400 * 1024 * 1024)
            .read_to_end(&mut bytes)?;
        let got = hex(&Sha256::digest(&bytes));
        if got != *sha {
            bail!(
                "{} {name}: {got}",
                tr("unexpected checksum for", "suma de verificación inesperada de")
            );
        }
        // written under another name and renamed: an interrupted download is never mistaken
        // for the model
        let part = dir.join(format!("{name}.part"));
        std::fs::File::create(&part)?.write_all(&bytes)?;
        std::fs::rename(&part, &target)?;
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn ort_err(e: impl std::fmt::Display) -> anyhow::Error {
    anyhow!("{e}")
}

pub struct Engine {
    tokenizer: tokenizers::Tokenizer,
    session: Session,
    wants_token_types: bool,
}

impl Engine {
    /// The tokenizer (the slower half) and the session load side by side.
    pub fn load() -> Result<Self> {
        let dir = model_dir()?;
        let tok_path = dir.join("tokenizer.json");
        let tok = std::thread::spawn(move || tokenizers::Tokenizer::from_file(tok_path).map_err(ort_err));
        // no graph optimisation: the model is already quantised, and it saves ~200 ms of load
        let mut builder = Session::builder()
            .map_err(ort_err)?
            .with_optimization_level(GraphOptimizationLevel::Disable)
            .map_err(ort_err)?
            .with_intra_threads(4)
            .map_err(ort_err)?;
        let session = builder
            .commit_from_file(dir.join("model.onnx"))
            .map_err(ort_err)?;
        let tokenizer = tok.join().map_err(|_| anyhow!("tokenizer thread panicked"))??;
        let wants_token_types = session.inputs().iter().any(|i| i.name() == "token_type_ids");
        Ok(Self {
            tokenizer,
            session,
            wants_token_types,
        })
    }

    /// One normalised vector per text. The e5 prefix (`query: ` / `passage: `) is the caller's.
    pub fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let mut encoded = Vec::with_capacity(texts.len());
        for t in texts {
            let enc = self.tokenizer.encode(t.as_str(), true).map_err(ort_err)?;
            let n = enc.get_ids().len().min(MAX_TOKENS);
            encoded.push((
                enc.get_ids()[..n].to_vec(),
                enc.get_attention_mask()[..n].to_vec(),
            ));
        }
        let len = encoded.iter().map(|(ids, _)| ids.len()).max().unwrap_or(1);
        let n = texts.len();
        // the model's padding id is 1 (<pad>) in XLM-R vocabularies
        let mut ids = vec![1i64; n * len];
        let mut mask = vec![0i64; n * len];
        for (row, (i, m)) in encoded.iter().enumerate() {
            for (col, (&id, &a)) in i.iter().zip(m).enumerate() {
                ids[row * len + col] = id as i64;
                mask[row * len + col] = a as i64;
            }
        }
        let mut inputs = vec![
            (
                "input_ids",
                Tensor::from_array(([n, len], ids)).map_err(ort_err)?.into_dyn(),
            ),
            (
                "attention_mask",
                Tensor::from_array(([n, len], mask.clone()))
                    .map_err(ort_err)?
                    .into_dyn(),
            ),
        ];
        if self.wants_token_types {
            inputs.push((
                "token_type_ids",
                Tensor::from_array(([n, len], vec![0i64; n * len]))
                    .map_err(ort_err)?
                    .into_dyn(),
            ));
        }
        let outputs = self.session.run(inputs).map_err(ort_err)?;
        let (shape, data) = outputs[0].try_extract_tensor::<f32>().map_err(ort_err)?;
        if shape.len() != 3 || shape[2] as usize != DIM {
            bail!("unexpected model output shape {shape:?}");
        }
        // mean pooling over the real tokens, then L2 normalisation
        let mut out = Vec::with_capacity(n);
        for row in 0..n {
            let mut v = vec![0f32; DIM];
            for col in 0..len {
                if mask[row * len + col] == 0 {
                    continue;
                }
                let base = (row * len + col) * DIM;
                for (acc, x) in v.iter_mut().zip(&data[base..base + DIM]) {
                    *acc += x;
                }
            }
            // normalising the sum is the same as normalising the mean
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
            for x in &mut v {
                *x /= norm;
            }
            out.push(v);
        }
        Ok(out)
    }

    pub fn embed_question(&mut self, question: &str) -> Result<Vec<f32>> {
        Ok(self.embed(&[format!("query: {question}")])?.remove(0))
    }
}

// ---------------------------------------------------------------- shared state

enum State {
    /// Embeddings are off, or there is no model: BM25 only.
    Off,
    Idle,
    Loading(JoinHandle<Result<Engine>>),
    Ready(Box<Engine>),
}

static STATE: Mutex<State> = Mutex::new(State::Off);

/// Decides, once per process, whether embeddings are used: `search.embeddings` in the config,
/// overridden by `PISTASHO_EMBEDDINGS=0|1`, and only if the model has been downloaded.
pub fn init(cfg: &Config) {
    let on = match std::env::var("PISTASHO_EMBEDDINGS").as_deref() {
        Ok("0" | "false" | "off") => false,
        Ok("1" | "true" | "on") => true,
        _ => cfg.search.embeddings,
    };
    *STATE.lock().unwrap() = if on && model_present() {
        State::Idle
    } else {
        State::Off
    };
}

pub fn enabled() -> bool {
    !matches!(*STATE.lock().unwrap(), State::Off)
}

/// Starts loading the model in the background. Cheap to call; does nothing twice.
pub fn prefetch() {
    let mut state = STATE.lock().unwrap();
    if matches!(*state, State::Idle) {
        *state = State::Loading(std::thread::spawn(Engine::load));
    }
}

/// Runs `f` with the model, waiting for the load if it is still going. `None` when embeddings
/// are off or the model does not load.
pub fn with_engine<R>(f: impl FnOnce(&mut Engine) -> R) -> Option<R> {
    let mut state = STATE.lock().unwrap();
    if matches!(*state, State::Idle) {
        *state = State::Loading(std::thread::spawn(Engine::load));
    }
    if matches!(*state, State::Loading(_)) {
        let State::Loading(handle) = std::mem::replace(&mut *state, State::Off) else {
            unreachable!()
        };
        // a failed load leaves the state `Off`: not retried within this process
        if let Ok(Ok(engine)) = handle.join() {
            *state = State::Ready(Box::new(engine));
        }
    }
    match &mut *state {
        State::Ready(engine) => Some(f(engine)),
        _ => None,
    }
}

/// The vector of a question, or `None` if embeddings are not available.
pub fn question_vector(question: &str) -> Option<Vec<f32>> {
    with_engine(|e| e.embed_question(question).ok()).flatten()
}

/// Embeds every entry that has no vector yet and stores them in batches, so an interrupted run
/// keeps what it did. `progress` is called with `(done, total)`. Returns how many were embedded.
pub fn index_missing(db: &crate::db::Db, mut progress: impl FnMut(usize, usize)) -> Result<usize> {
    if db.meta("embed_model")?.as_deref() != Some(MODEL_ID) {
        db.clear_vectors()?;
        db.set_meta("embed_model", MODEL_ID)?;
    }
    let missing = db.missing_vectors()?;
    let total = missing.len();
    if total == 0 {
        db.prune_vectors()?;
        return Ok(0);
    }
    let mut engine = Engine::load()?;
    let mut done = 0;
    for chunk in missing.chunks(BATCH) {
        let texts: Vec<String> = chunk.iter().map(|(_, p)| format!("passage: {p}")).collect();
        let vectors = engine.embed(&texts)?;
        let rows: Vec<(i64, Vec<i8>)> = chunk
            .iter()
            .zip(&vectors)
            .map(|((key, _), v)| (*key, semantic::quantize(v)))
            .collect();
        db.put_vectors(&rows)?;
        done += chunk.len();
        progress(done, total);
    }
    db.prune_vectors()?;
    Ok(total)
}
