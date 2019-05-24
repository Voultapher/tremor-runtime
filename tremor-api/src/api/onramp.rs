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

use crate::api::*;
use actix_web::{error, HttpRequest, Path};
use tremor_runtime::errors::*;

#[derive(Serialize)]
struct OnRampWrap {
    artefact: tremor_runtime::config::OnRamp,
    instances: Vec<String>,
}

pub fn list_artefact(req: HttpRequest<State>) -> ApiResult {
    let res: Result<Vec<String>> = req.state().world.repo.list_onramps().map(|l| {
        l.iter()
            .filter_map(tremor_runtime::url::TremorURL::artefact)
            .collect()
    });
    reply(req, res, false, 200)
}

pub fn publish_artefact((req, data_raw): (HttpRequest<State>, String)) -> ApiResult {
    let data: tremor_runtime::config::OnRamp = decode(&req, &data_raw)?;
    let url = build_url(&["onramp", &data.id])?;
    let res = req.state().world.repo.publish_onramp(url, false, data);
    reply(req, res, true, 201)
}

pub fn unpublish_artefact((req, id): (HttpRequest<State>, Path<(String)>)) -> ApiResult {
    let url = build_url(&["onramp", &id])?;
    let res = req.state().world.repo.unpublish_onramp(url);
    reply(req, res, true, 200)
}

pub fn get_artefact((req, id): (HttpRequest<State>, Path<String>)) -> ApiResult {
    let url = build_url(&["onramp", &id])?;
    let res = req
        .state()
        .world
        .repo
        .find_onramp(url)
        .map_err(|_e| error::ErrorInternalServerError("lookup failed"))?;
    match res {
        Some(res) => {
            let res: Result<OnRampWrap> = Ok(OnRampWrap {
                artefact: res.artefact,
                instances: res
                    .instances
                    .iter()
                    .filter_map(tremor_runtime::url::TremorURL::instance)
                    .collect(),
            });
            reply(req, res, false, 200)
        }
        None => Err(error::ErrorNotFound("Artefact not found")),
    }
}
