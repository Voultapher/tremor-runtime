// Copyright 2018-2019, Wayfair GmbH
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

//! # GCS Offramp
//!
//! The `GCS` offramp writes events to a GCS object. This offramp writes
//! exactly one object and finishes it when the pipeline is shut down.
//!
//! ## Configuration
//!
//! See [Config](struct.Config.html) for details.

use super::{Offramp, OfframpImpl};
use crate::codec::Codec;
use crate::dflt;
use crate::errors::*;
use crate::google::{self, storage_api, GcsHub};
use crate::offramp::prelude::make_postprocessors;
use crate::postprocessor::Postprocessors;
use crate::system::PipelineAddr;
use crate::url::TremorURL;
use crate::{Event, OpConfig};
use google_storage1::Object;
use hashbrown::HashMap;
use serde_yaml;
use std::io::Cursor;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Service Account secrets file ( json )
    pub service_account: String,
    /// bucket to write to
    pub bucket: String,
    /// object name
    pub name: String,
    /// Content encoding
    pub content_encoding: String,
    /// number of events in each batch
    pub batch_size: usize,
    /// Timeout before a batch is always send
    #[serde(default = "dflt::d_0")]
    pub timeout: u64,
}

/// An offramp that write to GCS
pub struct GCS {
    config: Config,
    hub: GcsHub,
    cnt: u64,
    pipelines: HashMap<TremorURL, PipelineAddr>,
    postprocessors: Postprocessors,
}

impl std::fmt::Debug for GCS {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::result::Result<(), std::fmt::Error> {
        write!(f, "(GcsHubOfframp, opaque)")
    }
}

impl OfframpImpl for GCS {
    fn from_config(config: &Option<OpConfig>) -> Result<Box<dyn Offramp>> {
        if let Some(config) = config {
            let config: Config = serde_yaml::from_value(config.clone())?;
            let hub = storage_api(&config.service_account.to_string())?;
            Ok(Box::new(GCS {
                cnt: 0,
                config,
                hub,
                pipelines: HashMap::new(),
                postprocessors: vec![],
            }))
        } else {
            Err("Missing config for gpub offramp".into())
        }
    }
}

impl Offramp for GCS {
    fn add_pipeline(&mut self, id: TremorURL, addr: PipelineAddr) {
        self.pipelines.insert(id, addr);
    }

    fn remove_pipeline(&mut self, id: TremorURL) -> bool {
        self.pipelines.remove(&id);
        self.pipelines.is_empty()
    }

    fn default_codec(&self) -> &str {
        "json"
    }

    fn start(&mut self, _codec: &Box<dyn Codec>, postprocessors: &[String]) {
        self.postprocessors = make_postprocessors(postprocessors)
            .expect("failed to setup post processors for stdout");
    }

    fn on_event(&mut self, codec: &Box<dyn Codec>, _input: String, event: Event) {
        for event in event.into_iter() {
            if let Ok(ref raw) = codec.encode(event.value) {
                let req = Object::default();
                let r = google::verbose(
                    self.hub
                        .objects()
                        .insert(req, &self.config.bucket)
                        .name(&format!("{}.{}", self.config.name, self.cnt))
                        .content_encoding(&self.config.content_encoding)
                        .upload(
                            Cursor::new(raw),
                            "application/octet-stream".parse().expect("parse ok"),
                        ),
                );
                self.cnt += 1;
                if let Err(ref e) = r {
                    error!("google cloud storage error {}: ", e);
                };
            }
        }
    }
}