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

#![cfg(not(tarpaulin_include))]

use crate::postprocessor::{make_postprocessors, postprocess, Postprocessors};
use crate::{codec::Codec, source::prelude::*};
use async_channel::{Sender, TryRecvError};
use async_std::net::{TcpListener, TcpStream};
use async_std::task;
use async_tungstenite::tungstenite::Message;
use futures::{SinkExt, StreamExt};
use halfbrown::HashMap;
use std::{collections::BTreeMap};
use tremor_pipeline::EventId;
use tremor_script::Value;

#[derive(Deserialize, Debug, Clone)]
pub struct Config {
    /// The port to listen on.
    pub port: u16,
    /// Host to listen on
    pub host: String,
}

impl ConfigImpl for Config {}

pub struct Tnt {
    pub config: Config,
    pub(crate) onramp_id: TremorURL,
}

impl onramp::Impl for Tnt {
    fn from_config(id: &TremorURL, config: &Option<YamlValue>) -> Result<Box<dyn Onramp>> {
        if let Some(config) = config {
            let config: Config = Config::new(config)?;
            Ok(Box::new(Self {
                config,
                onramp_id: id.clone(),
            }))
        } else {
            Err("Missing config for blaster onramp".into())
        }
    }
}

enum TntSourceReply {
    StartStream(usize, Option<Sender<SerializedResponse>>),
    EndStream(usize),
    Data(SourceReply), // stupid wrapper around SourceReply::Data
}

/// encoded response and additional information
/// for post-processing and assembling WS messages
pub struct SerializedResponse {
    pub(crate) event_id: EventId,
    pub(crate) ingest_ns: u64,
    pub(crate) data: Vec<u8>,
    pub(crate) binary: bool,
    pub(crate) should_close: bool,
}

pub struct TntImpl {
    uid: u64,
    onramp_id: TremorURL,
    config: Config,
    is_linked: bool,
    listener: Option<Receiver<TntSourceReply>>,
    post_processors: Vec<String>,
    // mapping of event id to stream id
    messages: BTreeMap<u64, usize>,
    // mapping of stream id to the stream sender
    // TODO alternative to this? possible to store actual ws_stream refs here?
    pub(crate) streams: BTreeMap<usize, Sender<SerializedResponse>>,
}

impl std::fmt::Debug for TntImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TNT")
    }
}

impl TntImpl {
    pub fn from_config(
        uid: u64,
        onramp_id: TremorURL,
        post_processors: &[String],
        config: &Config,
        is_linked: bool,
    ) -> Result<Self> {
        let config = config.clone();

        Ok(Self {
            uid,
            config,
            listener: None,
            post_processors: post_processors.to_vec(),
            onramp_id,
            is_linked,
            messages: BTreeMap::new(),
            streams: BTreeMap::new(),
        })
    }

    fn get_stream_sender_for_id(&self, id: u64) -> Option<&Sender<SerializedResponse>> {
        self.streams.get(&(id as usize))
    }
}

async fn handle_connection(
    source_url: TremorURL,
    tx: Sender<TntSourceReply>,
    raw_stream: TcpStream,
    origin_uri: EventOriginUri,
    processors: Vec<String>,
    stream: usize,
    link: bool,
) -> Result<()> {
    let ws_stream = async_tungstenite::accept_async(raw_stream).await?;
    // let close_stream = ws_stream.clone();
    let (mut ws_write, mut ws_read) = ws_stream.split();

    // TODO maybe send ws_write from tx and get rid of this task + extra channel?
    let stream_sender = if link {
        let (stream_tx, stream_rx): (Sender<SerializedResponse>, Receiver<SerializedResponse>) =
            bounded(crate::QSIZE);
        // response handling task
        task::spawn::<_, Result<()>>(async move {
            // create post-processors for this stream
            match make_postprocessors(processors.as_slice()) {
                Ok(mut post_processors) => {
                    // wait for response messages to arrive (via reply_event)
                    while let Ok(response) = stream_rx.recv().await {
                        let event_id = response.event_id.to_string();
                        let should_close = response.should_close;
                        let msgs = match make_messages(response, &mut post_processors) {
                            // post-process
                            Ok(messages) => messages,
                            Err(e) => {
                                error!(
                                    "[Source::{}] Error post-processing response event: {}",
                                    &source_url,
                                    e.to_string()
                                );
                                let err = create_error_response(
                                    format!("Error post-processing messages: {}", e),
                                    event_id,
                                    &source_url,
                                );
                                let mut msgs = Vec::with_capacity(1);
                                if let Ok(data) = simd_json::to_vec(&err) {
                                    msgs.push(Message::Binary(data));
                                } else {
                                    error!(
                                        "[Source::{}] Error serializing error response to json.",
                                        &source_url
                                    );
                                }
                                msgs
                            }
                        };
                        for msg in msgs {
                            ws_write.send(msg).await?;                            
                        }
                        if should_close {
                            break;
                        }
                    }
                }
                Err(e) => error!(
                    "[Onramp::Tnt] Invalid Post Processors, not starting response receiver task: {}",
                    e
                ), // shouldn't happen, got validated before in init and is not changes after
            }
            Ok(())
        });
        Some(stream_tx)
    } else {
        None
    };

    tx.send(TntSourceReply::StartStream(stream, stream_sender))
        .await?;

    while let Some(msg) = ws_read.next().await {
        let mut meta = Value::object_with_capacity(1);
        match msg {
            Ok(Message::Text(t)) => {
                meta.insert("binary", false)?;
                tx.send(TntSourceReply::Data(SourceReply::Data {
                    origin_uri: origin_uri.clone(),
                    data: t.into_bytes(),
                    meta: Some(meta),
                    codec_override: None,
                    stream,
                }))
                .await?;
            }
            Ok(Message::Binary(_data)) => {
                meta.insert("binary", true)?;
                // FIXME binary not supported for network protocol at this time
            }
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => (),
            Ok(Message::Close(_)) => {
                tx.send(TntSourceReply::EndStream(stream)).await?;
                break;
            }
            Err(e) => error!("WS error returned while waiting for client data: {}", e),
        }
    }

    Ok(())
}

fn make_messages(
    response: SerializedResponse,
    processors: &mut Postprocessors,
) -> Result<Vec<Message>> {
    let send_as_binary = response.binary;
    let processed = postprocess(processors.as_mut_slice(), response.ingest_ns, response.data)?;
    let mut processed: Vec<Message> = processed
        .into_iter()
        .map(|data| {
            if send_as_binary {
                Message::Binary(data)
            } else {
                let str = String::from_utf8_lossy(&data).to_string();
                Message::Text(str)
            }
        })
        .collect();

        if response.should_close {
        processed.push(Message::Close(None));
    }
    Ok(processed)
}

fn create_error_response(err: String, event_id: String, source_id: &TremorURL) -> Value<'static> {
    let mut err_data = tremor_value::Object::with_capacity(3);
    err_data.insert_nocheck("error".into(), Value::from(err));
    err_data.insert_nocheck("event_id".into(), Value::from(event_id));
    err_data.insert_nocheck("source_id".into(), Value::from(source_id.to_string()));
    Value::from(err_data)
}

#[async_trait::async_trait()]
impl Source for TntImpl {
    async fn pull_event(&mut self, id: u64) -> Result<SourceReply> {
        let messages = &mut self.messages;
        let streams = &mut self.streams;
        self.listener.as_ref().map_or_else(
            // listener channel dropped or not created yet, we are disconnected
            || Ok(SourceReply::StateChange(SourceState::Disconnected)),
            // get a request or other control message from the ws server
            |listener| match listener.try_recv() {
                Ok(r) => match r {
                    TntSourceReply::Data(wrapped) => match wrapped {
                        SourceReply::Data { stream, .. } => {
                            messages.insert(id, stream);
                            Ok(wrapped)
                        }
                        _ => Err(
                            "Invalid WsSourceReply received in pull_event. Something is fishy!"
                                .into(),
                        ),
                    },
                    TntSourceReply::StartStream(stream, ref sender) => {
                        if let Some(tx) = sender {
                            streams.insert(stream, tx.clone());
                        }
                        Ok(SourceReply::StartStream(stream))
                    }
                    TntSourceReply::EndStream(stream) => {
                        streams.remove(&stream);
                        Ok(SourceReply::EndStream(stream))
                    }
                },
                Err(TryRecvError::Empty) => Ok(SourceReply::Empty(10)),
                Err(TryRecvError::Closed) => {
                    Ok(SourceReply::StateChange(SourceState::Disconnected))
                }
            },
        )
    }

    async fn reply_event(
        &mut self,
        event: Event,
        codec: &dyn Codec,
        _codec_map: &HashMap<String, Box<dyn Codec>>,
    ) -> Result<()> {
        // FIXME reconsider event id mapping for network protocol
        //
        let eid = event.id.stream_id();
        if let Some(tx) = self.get_stream_sender_for_id(eid) {
            dbg!(&tx);
            dbg!(&event);
            for (value, meta) in event.value_meta_iter() {
                let binary = meta.get("binary").and_then(Value::as_bool).unwrap_or(false);
                // we do the encoding here, and the post-processing later on the sending task, as this is stream-based
                let data = match codec.encode(value) {
                    Ok(data) => data,
                    Err(e) => {
                        error!(
                            "[Source::{}] Error encoding reply event: {}",
                            &self.onramp_id,
                            e.to_string()
                        );
                        let err_res = create_error_response(
                            format!("Error encoding message: {}", e),
                            event.id.to_string(),
                            &self.onramp_id,
                        );
                        simd_json::to_vec(&err_res)? // for proper error handling
                    }
                };
                let res = SerializedResponse {
                    event_id: event.id.clone(),
                    ingest_ns: event.ingest_ns,
                    data,
                    binary,
                    should_close: false,
                };
                tx.send(res).await?;
            }
        }
        Ok(())
    }

    async fn init(&mut self) -> Result<SourceState> {
        let listen_host = &self.config.host;
        let listen_port = self.config.port;
        eprintln!("Network listening at: ws://{}:{}", listen_host, listen_port);
        info!("Network listening at: ws://{}:{}", listen_host, listen_port);
        let listener = TcpListener::bind((self.config.host.as_str(), listen_port)).await?;
        let (tx, rx) = bounded(crate::QSIZE);
        let uid = self.uid;
        let source_url = self.onramp_id.clone();

        let link = self.is_linked;

        make_postprocessors(self.post_processors.as_slice())?; // just for verification before starting the onramp
        let processors = self.post_processors.clone();
        task::spawn(async move {
            let mut stream_id = 0;
            while let Ok((stream, socket)) = listener.accept().await {
                let uri = EventOriginUri {
                    uid,
                    scheme: "tremor-tnt".to_string(),
                    host: socket.ip().to_string(),
                    port: Some(socket.port()),
                    path: vec![listen_port.to_string()],
                };

                stream_id += 1;
                task::spawn(handle_connection(
                    source_url.clone(),
                    tx.clone(),
                    stream,
                    uri,
                    processors.clone(),
                    stream_id,
                    link,
                ));
            }
        });

        self.listener = Some(rx);

        Ok(SourceState::Connected)
    }
    fn id(&self) -> &TremorURL {
        &self.onramp_id
    }
}

#[async_trait::async_trait]
impl Onramp for Tnt {
    async fn start(&mut self, config: OnrampConfig<'_>) -> Result<onramp::Addr> {
        let source = TntImpl::from_config(
            config.onramp_uid,
            self.onramp_id.clone(),
            config.processors.post,
            &self.config,
            config.is_linked,
        )?;
        SourceManager::start(source, config).await
    }

    fn default_codec(&self) -> &str {
        "json"
    }
}