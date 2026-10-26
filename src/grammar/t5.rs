//! A T5 model from Hugging Face, made to correct grammar: its files fetched
//! once into the cache, as letterbox fetches the web, read with candle, and
//! each sentence made over word by word, the likeliest each time.

use super::Corrector;
use anyhow::{Context, bail};
use candle_core::{D, DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::t5;
use std::path::PathBuf;
use tokenizers::Tokenizer;

/// The most tokens made of a sentence.
const LONGEST: usize = 256;
/// The largest file of a model fetched.
const LARGEST: u64 = 2 << 30;

pub struct Model {
    model: t5::T5ForConditionalGeneration,
    tokenizer: Tokenizer,
    config: t5::Config,
    /// What its sentences are begun with, as the model was taught: none for
    /// most, `gec: ` for Unbabel's.
    prefix: &'static str,
}

impl Model {
    /// The model `name`, on Hugging Face, read: its files fetched the first
    /// time, its weights as safetensors where it has them.
    pub fn open(name: &str) -> anyhow::Result<Model> {
        let config: t5::Config =
            serde_json::from_slice(&std::fs::read(fetched(name, "config.json")?)?)
                .with_context(|| format!("{name} is no T5 model letterbox reads"))?;
        let tokenizer = Tokenizer::from_file(fetched(name, "tokenizer.json")?)
            .map_err(|e| anyhow::anyhow!("{name}'s tokenizer: {e}"))?;
        let device = Device::Cpu;
        let vb = match fetched(name, "model.safetensors") {
            Ok(path) => {
                VarBuilder::from_buffered_safetensors(std::fs::read(path)?, DType::F32, &device)?
            }
            Err(_) => {
                VarBuilder::from_pth(fetched(name, "pytorch_model.bin")?, DType::F32, &device)?
            }
        };
        let model = t5::T5ForConditionalGeneration::load(vb, &config)?;
        let prefix = if name.starts_with("Unbabel/") {
            "gec: "
        } else {
            ""
        };
        Ok(Model {
            model,
            tokenizer,
            config,
            prefix,
        })
    }
}

impl Corrector for Model {
    fn correct(&mut self, sentence: &str) -> anyhow::Result<String> {
        let device = Device::Cpu;
        self.model.clear_kv_cache();
        let ids = self
            .tokenizer
            .encode(format!("{}{sentence}", self.prefix), true)
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .get_ids()
            .to_vec();
        let input = Tensor::new(ids.as_slice(), &device)?.unsqueeze(0)?;
        let encoded = self.model.encode(&input)?;
        let start = self
            .config
            .decoder_start_token_id
            .unwrap_or(self.config.pad_token_id) as u32;
        let mut made = vec![start];
        for i in 0..LONGEST {
            // The first step is given what is begun; each after, its last.
            let last = if i == 0 {
                &made[..]
            } else {
                &made[made.len() - 1..]
            };
            let decoder = Tensor::new(last, &device)?.unsqueeze(0)?;
            let logits = self.model.decode(&decoder, &encoded)?.squeeze(0)?;
            let next = logits.argmax(D::Minus1)?.to_scalar::<u32>()?;
            if next as usize == self.config.eos_token_id {
                break;
            }
            made.push(next);
        }
        self.tokenizer
            .decode(&made[1..], true)
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
}

/// Where a model's files are kept: `$XDG_CACHE_HOME/letterbox/models`,
/// else `~/.cache/letterbox/models`, each model in a folder of its name.
fn kept(name: &str) -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache")
        })
        .join("letterbox/models")
        .join(name.replace('/', "--"))
}

/// The file `file` of the model `name`: kept, or fetched from Hugging Face
/// and kept, whole or not at all.
fn fetched(name: &str, file: &str) -> anyhow::Result<PathBuf> {
    let dir = kept(name);
    let path = dir.join(file);
    if path.is_file() {
        return Ok(path);
    }
    let url = format!("https://huggingface.co/{name}/resolve/main/{file}");
    let mut response = crate::net::agent()
        .get(&url)
        .call()
        .with_context(|| format!("cannot fetch {name}'s {file}"))?;
    let status = response.status().as_u16();
    if status != 200 {
        bail!("{name} has no {file} ({status})");
    }
    let bytes = response
        .body_mut()
        .with_config()
        .limit(LARGEST)
        .read_to_vec()
        .with_context(|| format!("cannot fetch {name}'s {file}"))?;
    std::fs::create_dir_all(&dir)?;
    let part = dir.join(format!("{file}.part"));
    std::fs::write(&part, &bytes)?;
    std::fs::rename(&part, &path)?;
    Ok(path)
}
