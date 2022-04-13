// Copyright 2020-2021, The Tremor Team
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

mod gelf;
pub(crate) use gelf::Gelf;
pub(crate) mod join;

use crate::config::Postprocessor as PostprocessorConfig;
use crate::errors::Result;
use byteorder::{BigEndian, WriteBytesExt};
use std::default::Default;
use tremor_common::time::nanotime;
/// Set of Postprocessors
pub type Postprocessors = Vec<Box<dyn Postprocessor>>;
use std::io::Write;
use std::mem;
use std::str;

trait PostprocessorState {}
/// Postprocessor trait
pub trait Postprocessor: Send + Sync {
    /// Canonical name of the postprocessor
    fn name(&self) -> &str;
    /// process data
    ///
    /// # Errors
    ///
    ///   * Errors if the data could not be processed
    fn process(&mut self, ingres_ns: u64, egress_ns: u64, data: &[u8]) -> Result<Vec<Vec<u8>>>;

    /// Finish execution of this postprocessor.
    ///
    /// `data` is the result of the previous preprocessors `finish` execution if any,
    /// otherwise it is an empty slice.
    ///
    /// # Errors
    ///   * if the postprocessor could not be finished correctly
    fn finish(&mut self, _data: Option<&[u8]>) -> Result<Vec<Vec<u8>>> {
        Ok(vec![])
    }
}

/// Lookup a postprocessor via its config
///
/// # Errors
///
///   * Errors if the postprocessor is not known

pub fn lookup_with_config(config: &PostprocessorConfig) -> Result<Box<dyn Postprocessor>> {
    match config.name.as_str() {
        "join" => Ok(Box::new(join::Join::from_config(&config.config)?)),
        "lines" => Ok(Box::new(join::Join::default())),
        "base64" => Ok(Box::new(Base64::default())),
        "gzip" => Ok(Box::new(Gzip::default())),
        "zlib" => Ok(Box::new(Zlib::default())),
        "xz2" => Ok(Box::new(Xz2::default())),
        "snappy" => Ok(Box::new(Snappy::default())),
        "lz4" => Ok(Box::new(Lz4::default())),
        "ingest-ns" => Ok(Box::new(AttachIngresTs {})),
        "length-prefixed" => Ok(Box::new(LengthPrefix::default())),
        "gelf-chunking" => Ok(Box::new(Gelf::default())),
        "textual-length-prefix" => Ok(Box::new(TextualLength::default())),
        "zstd" => Ok(Box::new(Zstd::default())),
        name => Err(format!("Postprocessor '{}' not found.", name).into()),
    }
}

/// Lookup a postprocessor implementation via its unique name.
/// Only for backwards compatibility.
///
/// # Errors
///   * if the postprocessor with `name` is not known
pub fn lookup(name: &str) -> Result<Box<dyn Postprocessor>> {
    lookup_with_config(&PostprocessorConfig::from(name))
}

/// Given the slice of postprocessor names: Lookup each of them and return them as `Postprocessors`
///
/// # Errors
///
///   * If any postprocessor is not known.
pub fn make_postprocessors(postprocessors: &[PostprocessorConfig]) -> Result<Postprocessors> {
    postprocessors.iter().map(lookup_with_config).collect()
}

/// canonical way to process encoded data passed from a `Codec`
///
/// # Errors
///
///   * If a `Postprocessor` fails
pub fn postprocess(
    postprocessors: &mut [Box<dyn Postprocessor>], // We are borrowing a dyn box as we don't want to pass ownership.
    ingres_ns: u64,
    data: Vec<u8>,
    alias: &str,
) -> Result<Vec<Vec<u8>>> {
    let egress_ns = nanotime();
    let mut data = vec![data];
    let mut data1 = Vec::new();

    for pp in postprocessors {
        data1.clear();
        for d in &data {
            let mut r = pp
                .process(ingres_ns, egress_ns, d)
                .map_err(|e| format!("[Connector::{alias}] Postprocessor error {e}"))?;
            data1.append(&mut r);
        }
        mem::swap(&mut data, &mut data1);
    }

    Ok(data)
}

/// Canonical way to finish postprocessors up
///
/// # Errors
///
/// * If a postprocessor failed
pub fn finish(postprocessors: &mut [Box<dyn Postprocessor>], alias: &str) -> Result<Vec<Vec<u8>>> {
    if let Some((head, tail)) = postprocessors.split_first_mut() {
        let mut data = match head.finish(None) {
            Ok(d) => d,
            Err(e) => {
                error!(
                    "[Connector::{alias}] Postprocessor '{}' finish error: {e}",
                    head.name()
                );
                return Err(e);
            }
        };
        let mut data1 = Vec::new();
        for pp in tail {
            data1.clear();
            for d in &data {
                match pp.finish(Some(d)) {
                    Ok(mut r) => data1.append(&mut r),
                    Err(e) => {
                        error!(
                            "[Connector::{alias}] Postprocessor '{}' finish error: {e}",
                            pp.name()
                        );
                        return Err(e);
                    }
                }
            }
            std::mem::swap(&mut data, &mut data1);
        }
        Ok(data)
    } else {
        Ok(vec![])
    }
}

#[derive(Default)]
pub(crate) struct Base64 {}
impl Postprocessor for Base64 {
    fn name(&self) -> &str {
        "base64"
    }

    fn process(&mut self, _ingres_ns: u64, _egress_ns: u64, data: &[u8]) -> Result<Vec<Vec<u8>>> {
        Ok(vec![base64::encode(&data).as_bytes().to_vec()])
    }
}

#[derive(Default)]
pub(crate) struct Gzip {}
impl Postprocessor for Gzip {
    fn name(&self) -> &str {
        "gzip"
    }

    fn process(&mut self, _ingres_ns: u64, _egress_ns: u64, data: &[u8]) -> Result<Vec<Vec<u8>>> {
        use libflate::gzip::Encoder;

        let mut encoder = Encoder::new(Vec::new())?;
        encoder.write_all(data)?;
        Ok(vec![encoder.finish().into_result()?])
    }
}

#[derive(Default)]
pub(crate) struct Zlib {}
impl Postprocessor for Zlib {
    fn name(&self) -> &str {
        "zlib"
    }

    fn process(&mut self, _ingres_ns: u64, _egress_ns: u64, data: &[u8]) -> Result<Vec<Vec<u8>>> {
        use libflate::zlib::Encoder;
        let mut encoder = Encoder::new(Vec::new())?;
        encoder.write_all(data)?;
        Ok(vec![encoder.finish().into_result()?])
    }
}

#[derive(Default)]
pub(crate) struct Xz2 {}
impl Postprocessor for Xz2 {
    fn name(&self) -> &str {
        "xz2"
    }

    fn process(&mut self, _ingres_ns: u64, _egress_ns: u64, data: &[u8]) -> Result<Vec<Vec<u8>>> {
        use xz2::write::XzEncoder as Encoder;
        let mut encoder = Encoder::new(Vec::new(), 9);
        encoder.write_all(data)?;
        Ok(vec![encoder.finish()?])
    }
}

#[derive(Default)]
pub(crate) struct Snappy {}
impl Postprocessor for Snappy {
    fn name(&self) -> &str {
        "snappy"
    }

    fn process(&mut self, _ingres_ns: u64, _egress_ns: u64, data: &[u8]) -> Result<Vec<Vec<u8>>> {
        use snap::write::FrameEncoder;
        let mut writer = FrameEncoder::new(vec![]);
        writer.write_all(data)?;
        let compressed = writer
            .into_inner()
            .map_err(|e| format!("Snappy compression postprocessor error: {}", e))?;
        Ok(vec![compressed])
    }
}

#[derive(Default)]
pub(crate) struct Lz4 {}
impl Postprocessor for Lz4 {
    fn name(&self) -> &str {
        "lz4"
    }

    fn process(&mut self, _ingres_ns: u64, _egress_ns: u64, data: &[u8]) -> Result<Vec<Vec<u8>>> {
        use lz4::EncoderBuilder;
        let buffer = Vec::<u8>::new();
        let mut encoder = EncoderBuilder::new().level(4).build(buffer)?;
        encoder.write_all(data)?;
        Ok(vec![encoder.finish().0])
    }
}

pub(crate) struct AttachIngresTs {}
impl Postprocessor for AttachIngresTs {
    fn name(&self) -> &str {
        "attach-ingress-ts"
    }

    fn process(&mut self, ingres_ns: u64, _egress_ns: u64, data: &[u8]) -> Result<Vec<Vec<u8>>> {
        let mut res = Vec::with_capacity(data.len() + 8);
        res.write_u64::<BigEndian>(ingres_ns)?;
        res.write_all(data)?;

        Ok(vec![res])
    }
}

#[derive(Clone, Default)]
pub(crate) struct LengthPrefix {}
impl Postprocessor for LengthPrefix {
    fn name(&self) -> &str {
        "length-prefix"
    }

    fn process(&mut self, _ingres_ns: u64, _egress_ns: u64, data: &[u8]) -> Result<Vec<Vec<u8>>> {
        let mut res = Vec::with_capacity(data.len() + 8);
        res.write_u64::<BigEndian>(data.len() as u64)?;
        res.write_all(data)?;
        Ok(vec![res])
    }
}

#[derive(Clone, Default)]
pub(crate) struct TextualLength {}
impl Postprocessor for TextualLength {
    fn name(&self) -> &str {
        "textual-length-prefix"
    }

    fn process(&mut self, _ingres_ns: u64, _egress_ns: u64, data: &[u8]) -> Result<Vec<Vec<u8>>> {
        let size = data.len();
        let mut digits: Vec<u8> = size.to_string().into_bytes();
        let mut res = Vec::with_capacity(digits.len() + 1 + size);
        res.append(&mut digits);
        res.push(32);
        res.write_all(data)?;
        Ok(vec![res])
    }
}

#[derive(Clone, Default, Debug)]
pub(crate) struct Zstd {}
impl Postprocessor for Zstd {
    fn name(&self) -> &str {
        "zstd"
    }

    fn process(&mut self, _ingres_ns: u64, _egress_ns: u64, data: &[u8]) -> Result<Vec<Vec<u8>>> {
        // Value of 0 indicates default level for encode.
        let compressed = zstd::encode_all(data, 0)?;
        Ok(vec![compressed])
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const LOOKUP_TABLE: [&str; 12] = [
        "join",
        "base64",
        "gzip",
        "zlib",
        "xz2",
        "snappy",
        "lz4",
        "gelf-chunking",
        "ingest-ns",
        "length-prefixed",
        "textual-length-prefix",
        "zstd",
    ];

    #[test]
    fn test_lookup() -> Result<()> {
        for t in LOOKUP_TABLE.iter() {
            assert!(lookup(t).is_ok());
        }
        let t = "snot";
        assert!(lookup(&t).is_err());
        Ok(())
    }

    #[test]
    fn base64() -> Result<()> {
        let mut post = Base64 {};
        let data: [u8; 0] = [];

        assert_eq!(Ok(vec![vec![]]), post.process(0, 0, &data));

        assert_eq!(Ok(vec![b"Cg==".to_vec()]), post.process(0, 0, b"\n"));

        assert_eq!(Ok(vec![b"c25vdA==".to_vec()]), post.process(0, 0, b"snot"));

        assert!(post.finish(None)?.is_empty());
        Ok(())
    }

    #[test]
    fn textual_length_prefix_postp() -> Result<()> {
        let mut post = TextualLength {};
        let data = vec![1_u8, 2, 3];
        let encoded = post.process(42, 23, &data).unwrap().pop().unwrap();
        assert_eq!("3 \u{1}\u{2}\u{3}", str::from_utf8(&encoded).unwrap());
        assert!(post.finish(None)?.is_empty());
        Ok(())
    }
}
