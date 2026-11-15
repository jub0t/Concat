// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The whole contract as one JSON document: every request, what each
//! method replies with, every event a job raises and every error, with
//! one shared `$defs` of the types they carry. `concat --schema` prints
//! it, every release carries it, and the developer docs are built from
//! it, so the reference cannot drift from the build it describes.

use concat_host::session::EditorView;
use concat_host::{ProjectInfo, media::MediaSummary, templates::TemplateInfo};
use schemars::{JsonSchema, SchemaGenerator, generate::SchemaSettings};
use serde_json::{Map, Value, json};

use crate::message::{
    API_VERSION, ApiError, Done, ErrorCode, Event, JobStatus, PackageInfo, Picture, Request,
    Started, VersionInfo, Written,
};

/// Where the published document lives: the website (ConcatApp/website)
/// serves each release's copy at this path.
const ID: &str = "https://concat.standalones.org/concat-api.schema.json";

/// The contract document. Its shape:
///
/// - `apiVersion`, `concat`: the contract and the build that wrote it.
/// - `request`: one alternative per method, tagged by `method`.
/// - `replies`: each method's `result`, by method name.
/// - `event`, `error`: what a job raises, what a refusal carries.
/// - `$defs`: every named type the others refer to, once.
pub fn contract_schema() -> Value {
    let mut generator = SchemaSettings::draft2020_12().into_generator();
    let request = generator.subschema_for::<Request>();
    let event = generator.subschema_for::<Event>();
    let error = generator.subschema_for::<ApiError>();
    let codes = generator.subschema_for::<ErrorCode>();
    let mut replies = Map::new();
    for method in methods(&request, &generator) {
        let reply = reply_schema(&method, &mut generator)
            .unwrap_or_else(|| panic!("no reply stated for `{method}`"));
        replies.insert(method, reply);
    }
    let defs = generator.take_definitions(true);
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": ID,
        "title": "Concat API",
        "apiVersion": API_VERSION,
        "concat": env!("CARGO_PKG_VERSION"),
        "request": request,
        "replies": replies,
        "event": event,
        "error": error,
        "errorCode": codes,
        "$defs": defs,
    })
}

/// What `method` replies with, or `None` for a name the API does not
/// answer. A method added to [`Request`] without a line here fails the
/// contract's test, so no method goes out undocumented.
fn reply_schema(method: &str, generator: &mut SchemaGenerator) -> Option<Value> {
    fn of<T: JsonSchema>(generator: &mut SchemaGenerator) -> Value {
        generator.subschema_for::<T>().to_value()
    }
    Some(match method {
        "version" => of::<VersionInfo>(generator),
        "project.create"
        | "project.open"
        | "project.get"
        | "project.setVideo"
        | "edit.apply"
        | "edit.undo"
        | "edit.redo"
        | "media.import"
        | "template.instantiate" => of::<EditorView>(generator),
        "project.close" | "project.save" | "export.cancel" => of::<Done>(generator),
        "project.list" => of::<Vec<ProjectInfo>>(generator),
        "project.document" => json!({
            "type": "object",
            "description": "The `concat.json` a save would write, as it stands."
        }),
        "media.probe" => of::<MediaSummary>(generator),
        "catalogue.list" => of::<Vec<PackageInfo>>(generator),
        "template.list" => of::<Vec<TemplateInfo>>(generator),
        "template.save" => of::<TemplateInfo>(generator),
        "export.run" => of::<Started>(generator),
        "export.status" => of::<JobStatus>(generator),
        "preview.frame" => json!({
            "description": "The file written when the request names an `output`; the picture itself when it does not.",
            "oneOf": [of::<Written>(generator), of::<Picture>(generator)]
        }),
        _ => return None,
    })
}

/// Every method name the request schema tags.
fn methods(request: &schemars::Schema, generator: &SchemaGenerator) -> Vec<String> {
    let root = request.as_value();
    let resolved = root
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| reference.strip_prefix("#/$defs/"))
        .and_then(|name| generator.definitions().get(name))
        .unwrap_or(root);
    resolved
        .get("oneOf")
        .or_else(|| resolved.get("anyOf"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|variant| {
            variant
                .pointer("/properties/method/const")
                .or_else(|| variant.pointer("/properties/method/enum/0"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_method_has_a_reply_and_every_reference_resolves() {
        let contract = contract_schema();
        let replies = contract["replies"].as_object().expect("replies");
        let request = &contract["$defs"]["Request"];
        let variants = request
            .get("oneOf")
            .or_else(|| request.get("anyOf"))
            .and_then(Value::as_array)
            .expect("one alternative per method");
        assert_eq!(replies.len(), variants.len(), "every method is named");
        assert!(replies.contains_key("edit.apply") && replies.contains_key("preview.frame"));
        let defs = contract["$defs"].as_object().expect("defs");
        for name in [
            "Command",
            "EditorView",
            "Project",
            "Clip",
            "ShapeStyle",
            "Event",
        ] {
            assert!(defs.contains_key(name), "{name} is defined");
        }
        let text = contract.to_string();
        for reference in text.split("\"$ref\":\"#/$defs/").skip(1) {
            let name = &reference[..reference.find('"').expect("closed")];
            assert!(
                defs.contains_key(name),
                "{name} is referred to but not defined"
            );
        }
    }

    #[test]
    fn a_reply_names_no_method_the_api_lacks() {
        let mut generator = SchemaSettings::draft2020_12().into_generator();
        assert!(reply_schema("project.delete", &mut generator).is_none());
    }
}
