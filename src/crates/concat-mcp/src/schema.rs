// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Tool input schemas, cut out of the API's own.
//!
//! [`concat_api::request_schema`] is the schema of every request, made by
//! `schemars` from the types: one alternative per method, and every type
//! a request carries under `$defs`. A tool that is a method takes that
//! method's alternative with `method` taken out; a tool that is one edit
//! command takes that command's alternative with `op` taken out and the
//! project's `path` put in. Nothing here describes a field: a field's
//! type, range and description are what its Rust type and doc comment
//! say, so a tool's schema changes when - and only when - the API does.
//!
//! References are inlined, because not every MCP client follows `$ref`;
//! the one that cannot be, the edit command a batch nests inside itself,
//! stays a reference with its definition carried along under `$defs`.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

/// The API's request schema, ready to cut tools out of.
pub struct Schemas {
    /// Every type a request carries, by name.
    defs: Map<String, Value>,
    /// Each method's alternative.
    methods: Vec<Value>,
}

impl Schemas {
    /// Generates the API's schema and indexes it.
    pub fn generate() -> Schemas {
        Schemas::from_root(concat_api::request_schema())
    }

    fn from_root(mut root: Value) -> Schemas {
        let defs = match root.get_mut("$defs").map(Value::take) {
            Some(Value::Object(defs)) => defs,
            _ => Map::new(),
        };
        let methods = alternatives(&root);
        Schemas { defs, methods }
    }

    /// The arguments of `method`, as a self-contained schema: the
    /// method's params, without `method`. `None` for a method the API does
    /// not have.
    pub fn method(&self, method: &str) -> Option<Value> {
        let variant = self
            .methods
            .iter()
            .find(|variant| tag_of(variant, "method") == Some(method))?;
        Some(self.finish(without(variant.clone(), "method")))
    }

    /// The arguments of the edit command `op`, plus the project's `path`
    /// it is applied to. `None` for a command the API does not have.
    pub fn command(&self, op: &str) -> Option<Value> {
        let variant = self
            .commands()
            .into_iter()
            .find(|variant| tag_of(variant, "op") == Some(op))?;
        let mut schema = without(variant, "op");
        add_required(&mut schema, "path", self.path_property());
        Some(self.finish(schema))
    }

    /// One type's schema by its Rust name - a `TextStyle`, say - inlined,
    /// for a tool that builds its own arguments around it.
    pub fn def(&self, name: &str) -> Option<Value> {
        let def = self.defs.get(name)?.clone();
        let mut kept = BTreeSet::new();
        let inlined = inline(def, &self.defs, &mut vec![name.to_owned()], &mut kept);
        // A type that nests itself would need its definitions carried; none
        // a tool builds around does, and `finish` is for whole schemas.
        Some(inlined)
    }

    /// Every edit command's alternative.
    fn commands(&self) -> Vec<Value> {
        self.defs
            .get("Command")
            .map(alternatives)
            .unwrap_or_default()
    }

    /// The `path` every project method takes, as the API describes it.
    fn path_property(&self) -> Value {
        self.method("project.get")
            .and_then(|schema| schema.pointer("/properties/path").cloned())
            .unwrap_or_else(|| json!({ "type": "string", "description": "The project folder." }))
    }

    /// Inlines every reference that can be, and carries the definitions of
    /// those that cannot. The carried ones - the whole edit vocabulary,
    /// for a tool that takes any command - are kept as the API writes
    /// them, each type once and referred to, with every description cut
    /// to its first sentence: complete, and a few thousand tokens of an
    /// agent's context rather than twenty.
    fn finish(&self, schema: Value) -> Value {
        let mut kept = BTreeSet::new();
        let mut schema = inline(schema, &self.defs, &mut Vec::new(), &mut kept);
        if schema.get("type").is_none() {
            schema["type"] = json!("object");
        }
        if !kept.is_empty() {
            let mut defs = Map::new();
            let mut pending: Vec<String> = kept.into_iter().collect();
            while let Some(name) = pending.pop() {
                if defs.contains_key(&name) {
                    continue;
                }
                let Some(def) = self.defs.get(&name) else {
                    continue;
                };
                let body = compact(def.clone());
                pending.extend(
                    references(&body)
                        .into_iter()
                        .filter(|other| !defs.contains_key(other)),
                );
                defs.insert(name, body);
            }
            schema["$defs"] = Value::Object(defs);
        }
        schema
    }
}

/// Every `#/$defs/` name `schema` refers to.
fn references(schema: &Value) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut stack = vec![schema];
    while let Some(value) = stack.pop() {
        match value {
            Value::Object(object) => {
                if let Some(Value::String(reference)) = object.get("$ref")
                    && let Some(name) = reference.strip_prefix("#/$defs/")
                {
                    found.insert(name.to_owned());
                }
                stack.extend(object.values());
            }
            Value::Array(items) => stack.extend(items.iter()),
            _ => {}
        }
    }
    found
}

/// A definition made short: descriptions to their first sentence, and the
/// defaults that are whole objects - a style's every field - left out,
/// the field's own description saying what absent means.
fn compact(schema: Value) -> Value {
    match schema {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .filter(|(key, value)| {
                    !(key == "default" && (value.is_object() || value.is_array()))
                })
                .map(|(key, value)| match (key.as_str(), value) {
                    ("description", Value::String(text)) => {
                        (key, Value::String(first_sentence(&text)))
                    }
                    (_, value) => (key, compact(value)),
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(compact).collect()),
        other => other,
    }
}

/// The first sentence of a doc comment, on one line.
fn first_sentence(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.find(". ") {
        Some(end) => flat[..=end].to_owned(),
        None => flat,
    }
}

/// The alternatives of a tagged enum's schema: `oneOf`, or `anyOf`.
fn alternatives(schema: &Value) -> Vec<Value> {
    ["oneOf", "anyOf"]
        .iter()
        .find_map(|key| schema.get(*key).and_then(Value::as_array))
        .cloned()
        .unwrap_or_default()
}

/// The constant an alternative's tag property holds.
fn tag_of<'a>(variant: &'a Value, tag: &str) -> Option<&'a str> {
    let property = variant.pointer(&format!("/properties/{tag}"))?;
    property.get("const").and_then(Value::as_str).or_else(|| {
        property
            .get("enum")
            .and_then(Value::as_array)
            .and_then(|values| values.first())
            .and_then(Value::as_str)
    })
}

/// An alternative without its tag: the property and its requirement.
fn without(mut variant: Value, tag: &str) -> Value {
    if let Some(properties) = variant.get_mut("properties").and_then(Value::as_object_mut) {
        properties.remove(tag);
    }
    if let Some(required) = variant.get_mut("required").and_then(Value::as_array_mut) {
        required.retain(|name| name.as_str() != Some(tag));
        if required.is_empty() {
            variant
                .as_object_mut()
                .map(|object| object.remove("required"));
        }
    }
    if variant.get("properties").is_none() {
        variant["properties"] = json!({});
    }
    variant
}

/// Adds a required property to an object schema.
fn add_required(schema: &mut Value, name: &str, property: Value) {
    schema["properties"][name] = property;
    let mut names = vec![json!(name)];
    if let Some(Value::Array(rest)) = schema.get_mut("required").map(Value::take) {
        names.extend(
            rest.into_iter()
                .filter(|other| other.as_str() != Some(name)),
        );
    }
    schema["required"] = Value::Array(names);
}

/// The types that stay references, defined once under `$defs`: the edit
/// command, which nests inside itself and is large enough that a tool
/// taking one should not carry it twice.
const SHARED: &[&str] = &["Command"];

/// `schema` with every `#/$defs/` reference replaced by its definition,
/// except one to a [`SHARED`] type or inside its own definition - which
/// stays, and is noted in `kept` - and with the `$schema` and `$defs` of a
/// root dropped.
fn inline(
    schema: Value,
    defs: &Map<String, Value>,
    stack: &mut Vec<String>,
    kept: &mut BTreeSet<String>,
) -> Value {
    match schema {
        Value::Object(mut object) => {
            object.remove("$schema");
            object.remove("$defs");
            if let Some(Value::String(reference)) = object.get("$ref")
                && let Some(name) = reference.strip_prefix("#/$defs/")
            {
                let name = name.to_owned();
                if SHARED.contains(&name.as_str())
                    || stack.contains(&name)
                    || !defs.contains_key(&name)
                {
                    kept.insert(name);
                    return Value::Object(object);
                }
                object.remove("$ref");
                stack.push(name.clone());
                let body = inline(defs[&name].clone(), defs, stack, kept);
                stack.pop();
                // What sat beside the reference - a field's own
                // description, its default - wins over the type's.
                let mut merged = match body {
                    Value::Object(body) => body,
                    other => return other,
                };
                for (key, value) in object {
                    let value = inline(value, defs, stack, kept);
                    merged.insert(key, value);
                }
                return Value::Object(merged);
            }
            Value::Object(
                object
                    .into_iter()
                    .map(|(key, value)| (key, inline(value, defs, stack, kept)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| inline(item, defs, stack, kept))
                .collect(),
        ),
        other => other,
    }
}

/// A small JSON Schema checker: enough of the 2020-12 vocabulary that
/// schemars writes - types, properties, required, items, enums and
/// constants, the three combinators, local references - to test that what
/// a tool's schema accepts is what its Rust type reads, and back.
#[cfg(test)]
pub(crate) fn check(schema: &Value, value: &Value, root: &Value, at: &str) -> Result<(), String> {
    let fail = |why: String| Err(format!("{at}: {why}"));
    let Some(object) = schema.as_object() else {
        return match schema {
            Value::Bool(false) => fail("nothing is allowed here".to_owned()),
            _ => Ok(()),
        };
    };
    if let Some(Value::String(reference)) = object.get("$ref") {
        let name = reference.trim_start_matches("#/$defs/");
        let Some(def) = root.pointer(&format!("/$defs/{name}")) else {
            return fail(format!("no definition {name}"));
        };
        check(def, value, root, at)?;
    }
    if let Some(kind) = object.get("type") {
        let kinds: Vec<&str> = match kind {
            Value::String(kind) => vec![kind.as_str()],
            Value::Array(kinds) => kinds.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        let fits = |kind: &str| match kind {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            "number" => value.is_number(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            _ => true,
        };
        if !kinds.iter().any(|kind| fits(kind)) {
            return fail(format!("{value} is not {kinds:?}"));
        }
    }
    if let Some(constant) = object.get("const")
        && constant != value
    {
        return fail(format!("{value} is not {constant}"));
    }
    if let Some(Value::Array(allowed)) = object.get("enum")
        && !allowed.contains(value)
    {
        return fail(format!("{value} is not one of {allowed:?}"));
    }
    if let Some(number) = value.as_f64() {
        if let Some(minimum) = object.get("minimum").and_then(Value::as_f64)
            && number < minimum
        {
            return fail(format!("{number} is under {minimum}"));
        }
        if let Some(maximum) = object.get("maximum").and_then(Value::as_f64)
            && number > maximum
        {
            return fail(format!("{number} is over {maximum}"));
        }
    }
    if let Value::Object(fields) = value {
        let properties = object.get("properties").and_then(Value::as_object);
        if let Some(Value::Array(required)) = object.get("required") {
            for name in required.iter().filter_map(Value::as_str) {
                if !fields.contains_key(name) {
                    return fail(format!("{name} is required"));
                }
            }
        }
        for (name, field) in fields {
            match properties.and_then(|properties| properties.get(name)) {
                Some(property) => check(property, field, root, &format!("{at}.{name}"))?,
                None => {
                    if let Some(extra) = object.get("additionalProperties") {
                        check(extra, field, root, &format!("{at}.{name}"))?;
                    } else if properties.is_some()
                        && object.get("oneOf").is_none()
                        && object.get("anyOf").is_none()
                        && object.get("allOf").is_none()
                    {
                        return fail(format!("{name} is not a field the schema knows"));
                    }
                }
            }
        }
    }
    if let (Value::Array(items), Some(item)) = (value, object.get("items")) {
        for (index, element) in items.iter().enumerate() {
            check(item, element, root, &format!("{at}[{index}]"))?;
        }
    }
    if let Some(Value::Array(all)) = object.get("allOf") {
        for part in all {
            check(part, value, root, at)?;
        }
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(Value::Array(choices)) = object.get(key) {
            let fitting = choices
                .iter()
                .filter(|choice| check(choice, value, root, at).is_ok())
                .count();
            let ok = if key == "oneOf" {
                fitting == 1
            } else {
                fitting >= 1
            };
            if !ok {
                let why: Vec<String> = choices
                    .iter()
                    .filter_map(|choice| check(choice, value, root, at).err())
                    .take(3)
                    .collect();
                return fail(format!("{fitting} alternatives of {key} fit: {why:?}"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_method_is_cut_out_without_its_tag_and_with_its_references_inlined() {
        let schemas = Schemas::generate();
        let open = schemas.method("project.open").expect("a method");
        assert_eq!(open["type"], "object");
        assert!(open["properties"].get("method").is_none());
        assert_eq!(open["required"], json!(["path"]));
        assert!(schemas.method("project.nope").is_none());
        let set_video = schemas.method("project.setVideo").expect("a method");
        assert!(
            set_video
                .pointer("/properties/video/properties/width")
                .is_some(),
            "VideoSettings inlined: {set_video}"
        );
        assert!(!set_video.to_string().contains("$ref"));
    }

    #[test]
    fn a_command_carries_the_project_path() {
        let schemas = Schemas::generate();
        let trim = schemas.command("trimClip").expect("a command");
        assert_eq!(
            trim.pointer("/properties/path/type"),
            Some(&json!("string"))
        );
        assert!(trim["properties"].get("op").is_none());
        let required = trim["required"].as_array().expect("required");
        assert!(required.contains(&json!("path")) && required.contains(&json!("clipId")));
        assert!(schemas.command("doTheThing").is_none());
    }

    #[test]
    fn the_tool_list_fits_an_agents_context() {
        // Every tool's schema sits in the agent's context for the whole
        // session. Measured at about 100 KB in all, 41 KB of it the whole
        // edit vocabulary under edit_apply; a type that grows past these
        // is worth a look before it ships.
        let toolbox = crate::tools::Toolbox::new();
        let sizes: Vec<(String, usize)> = toolbox
            .list()
            .iter()
            .map(|tool| (tool["name"].to_string(), tool.to_string().len()))
            .collect();
        let total: usize = sizes.iter().map(|(_, size)| size).sum();
        assert!(total < 160_000, "{total}: {sizes:?}");
        for (name, size) in &sizes {
            assert!(*size < 64_000, "{name}: {size}");
        }
    }

    #[test]
    fn a_batch_keeps_the_one_reference_it_must_and_carries_its_definition() {
        let schemas = Schemas::generate();
        let batch = schemas.command("batch").expect("a command");
        assert_eq!(
            batch.pointer("/properties/commands/items/$ref"),
            Some(&json!("#/$defs/Command"))
        );
        let command = batch.pointer("/$defs/Command").expect("carried");
        assert!(
            command
                .get("oneOf")
                .or_else(|| command.get("anyOf"))
                .is_some()
        );
        let sample = json!({
            "path": "/p",
            "commands": [
                { "op": "addTrack" },
                { "op": "batch", "commands": [{ "op": "addTimeline" }] }
            ]
        });
        check(&batch, &sample, &batch, "batch").expect("fits");
    }
}
