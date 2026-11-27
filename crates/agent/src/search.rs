//! Elasticsearch and OpenSearch. A dump is one line per record: an index with its settings
//! and mappings, followed by its documents.

use std::{
    io::{BufRead, Write},
    time::Duration,
};

use dbm_shared::jobs::{DatabaseSecret, SEARCH_ALL, valid_index_name};
use reqwest::{Client, Method, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json, value::RawValue};

use crate::Result;

/// Documents per page when reading, and the most sent in one bulk request.
const PAGE: usize = 500;
const BULK_BYTES: usize = 5 * 1024 * 1024;
/// How long the server keeps a read position between two pages.
const KEEP: &str = "10m";

/// Settings the server sets itself and refuses on a new index.
const OWN_SETTINGS: [&str; 8] = [
    "uuid",
    "creation_date",
    "version",
    "provided_name",
    "history_uuid",
    "resize",
    "verified_before_close",
    "blocks",
];

#[derive(Serialize, Deserialize)]
struct Index {
    dbm_index: String,
    settings: Value,
    mappings: Value,
    aliases: Value,
}

#[derive(Serialize, Deserialize)]
struct Document {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    routing: Option<String>,
    source: Box<RawValue>,
}

#[derive(Deserialize)]
struct Hit {
    #[serde(rename = "_id")]
    id: String,
    #[serde(rename = "_routing", default)]
    routing: Option<String>,
    #[serde(rename = "_source", default)]
    source: Option<Box<RawValue>>,
}

#[derive(Deserialize)]
struct Page {
    #[serde(rename = "_scroll_id")]
    scroll: String,
    #[serde(rename = "_shards")]
    shards: Shards,
    hits: Hits,
}

#[derive(Deserialize)]
struct Shards {
    failed: u64,
}

#[derive(Deserialize)]
struct Hits {
    hits: Vec<Hit>,
}

fn causes(error: reqwest::Error) -> String {
    let mut text = error.to_string();
    let mut next = std::error::Error::source(&error);
    while let Some(cause) = next {
        text = format!("{text}: {cause}");
        next = cause.source();
    }
    text
}

struct Api {
    client: Client,
    base: Url,
    username: String,
    password: String,
    runtime: tokio::runtime::Runtime,
}

impl Api {
    fn new(secret: &DatabaseSecret) -> Result<Self> {
        let (scheme, verified) = match secret.ssl_mode.as_str() {
            "disable" | "allow" | "prefer" => ("http", true),
            "require" => ("https", false),
            "verify-ca" | "verify-full" => ("https", true),
            _ => return Err("unknown ssl mode".into()),
        };
        // set_host refuses anything that is not just a host name.
        let mut base = Url::parse(&format!("{scheme}://localhost"))?;
        base.set_host(Some(&secret.host))
            .map_err(|_| "invalid host name")?;
        base.set_port(Some(secret.port))
            .map_err(|_| "invalid port")?;
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(600))
            // The password must not follow a redirect to another place.
            .redirect(reqwest::redirect::Policy::none())
            .danger_accept_invalid_certs(!verified)
            .build()?;
        Ok(Self {
            client,
            base,
            username: secret.username.clone(),
            password: secret.password.clone(),
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
        })
    }

    /// `path` is made of fixed words and checked index names only.
    fn send(&self, method: Method, path: &str, body: Option<(&str, Vec<u8>)>) -> Result<Vec<u8>> {
        let mut request = self.client.request(method.clone(), self.base.join(path)?);
        if !self.username.is_empty() {
            request = request.basic_auth(&self.username, Some(&self.password));
        }
        if let Some((kind, body)) = body {
            request = request.header("content-type", kind).body(body);
        }
        self.runtime.block_on(async {
            // The reason (a refused certificate, a closed port) sits in the causes.
            let reply = request.send().await.map_err(causes)?;
            let status = reply.status();
            let bytes = reply.bytes().await?;
            if status.is_success() {
                return Ok(bytes.to_vec());
            }
            let said: String = String::from_utf8_lossy(&bytes).chars().take(400).collect();
            let place = path.split('?').next().unwrap_or(path);
            Err(format!("{method} /{place} answered {status}: {said}").into())
        })
    }

    fn json(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        let body = body.map(|v| ("application/json", v.to_string().into_bytes()));
        Ok(serde_json::from_slice(&self.send(method, path, body)?)?)
    }
}

pub fn test(secret: &DatabaseSecret) -> Result<String> {
    let info = Api::new(secret)?.json(Method::GET, "", None)?;
    let version = &info["version"];
    let product = match version["distribution"].as_str() {
        Some("opensearch") => "OpenSearch",
        _ => "Elasticsearch",
    };
    let number = version["number"]
        .as_str()
        .ok_or("this does not answer like a search engine")?;
    Ok(format!("{product} {number}"))
}

/// What to send when creating the index again.
fn definition(name: &str, mut found: Value) -> Index {
    let mut index = found[name].take();
    if let Some(settings) = index["settings"]["index"].as_object_mut() {
        for key in OWN_SETTINGS {
            settings.remove(key);
        }
    }
    Index {
        dbm_index: name.into(),
        settings: index["settings"].take(),
        mappings: index["mappings"].take(),
        aliases: index["aliases"].take(),
    }
}

/// Hidden indices, and the ones OpenSearch writes about itself: it recreates those while
/// a restore is running.
fn servers_own(name: &str) -> bool {
    name.starts_with('.')
        || ["top_queries-", "security-auditlog-"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

/// Every index that is not hidden or belonging to the server.
fn all_indices(api: &Api) -> Result<Vec<String>> {
    let rows = api.json(
        Method::GET,
        "_cat/indices?format=json&h=index&expand_wildcards=open",
        None,
    )?;
    let mut names = Vec::new();
    for row in rows.as_array().ok_or("unreadable index list")? {
        let name = row["index"].as_str().unwrap_or_default();
        if servers_own(name) {
            continue;
        }
        if valid_index_name(name) {
            names.push(name.to_owned());
        } else {
            // Left out, and said so: a backup must not look complete when it is not.
            return Err(format!("index {name} has a name this backup cannot handle").into());
        }
    }
    names.sort();
    Ok(names)
}

/// A document may have been stored with line breaks; in valid JSON they only ever sit
/// between tokens, so a space means the same.
fn one_line(source: Box<RawValue>) -> Result<Box<RawValue>> {
    if !source.get().contains(['\n', '\r']) {
        return Ok(source);
    }
    Ok(RawValue::from_string(
        source.get().replace(['\n', '\r'], " "),
    )?)
}

fn line(out: &mut impl Write, record: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *out, record)?;
    out.write_all(b"\n")?;
    Ok(())
}

fn documents(api: &Api, index: &str, out: &mut impl Write) -> Result<u64> {
    // Only searchable documents are read.
    api.send(Method::POST, &format!("{index}/_refresh"), None)?;
    let first = json!({ "size": PAGE, "sort": ["_doc"] }).to_string();
    let mut bytes = api.send(
        Method::POST,
        &format!("{index}/_search?scroll={KEEP}"),
        Some(("application/json", first.into_bytes())),
    )?;
    let mut count = 0;
    loop {
        let page: Page = serde_json::from_slice(&bytes)?;
        if page.shards.failed > 0 {
            return Err(format!("{index}: some shards could not be read").into());
        }
        if page.hits.hits.is_empty() {
            let done = json!({ "scroll_id": page.scroll });
            // Left open it would only expire later.
            let _ = api.json(Method::DELETE, "_search/scroll", Some(done));
            return Ok(count);
        }
        for hit in page.hits.hits {
            let source = hit
                .source
                .ok_or_else(|| format!("{index} does not keep _source, so it cannot be dumped"))?;
            line(
                out,
                &Document {
                    id: hit.id,
                    routing: hit.routing,
                    source: one_line(source)?,
                },
            )?;
            count += 1;
        }
        let next = json!({ "scroll": KEEP, "scroll_id": page.scroll }).to_string();
        bytes = api.send(
            Method::POST,
            "_search/scroll",
            Some(("application/json", next.into_bytes())),
        )?;
    }
}

pub fn dump(secret: &DatabaseSecret, name: &str, out: impl Write) -> Result<()> {
    let api = Api::new(secret)?;
    let names = if name == SEARCH_ALL {
        all_indices(&api)?
    } else if valid_index_name(name) {
        vec![name.to_owned()]
    } else {
        return Err("invalid index name".into());
    };
    let mut out = std::io::BufWriter::new(out);
    for name in &names {
        let found = api.json(Method::GET, name, None)?;
        // An alias answers with the index behind it, under another name.
        if !found[name.as_str()].is_object() {
            return Err(format!("{name} is not an index (an alias or data stream?)").into());
        }
        line(&mut out, &definition(name, found))?;
        let count = documents(&api, name, &mut out)?;
        eprintln!("{name}: {count} documents");
    }
    out.flush()?;
    Ok(())
}

/// Deletes the index if it is there and creates it from the dump.
fn create(api: &Api, target: &str, index: &Index) -> Result<()> {
    api.send(
        Method::DELETE,
        &format!("{target}?ignore_unavailable=true"),
        None,
    )?;
    let mut body = json!({ "settings": index.settings, "mappings": index.mappings });
    // Under another name, copying the aliases would put the copy behind them too.
    if target == index.dbm_index {
        body["aliases"] = index.aliases.clone();
    }
    api.json(Method::PUT, target, Some(body))?;
    Ok(())
}

fn bulk(api: &Api, target: &str, batch: &mut Vec<u8>) -> Result<()> {
    if batch.is_empty() {
        return Ok(());
    }
    let reply = api.send(
        Method::POST,
        &format!("{target}/_bulk"),
        Some(("application/x-ndjson", std::mem::take(batch))),
    )?;
    let reply: Value = serde_json::from_slice(&reply)?;
    if reply["errors"] == json!(false) {
        return Ok(());
    }
    let first = reply["items"]
        .as_array()
        .and_then(|items| items.iter().find_map(|i| i["index"].get("error")));
    Err(format!("{target} refused documents: {}", first.unwrap_or(&reply)).into())
}

pub fn restore(
    secret: &DatabaseSecret,
    name: &str,
    source: &str,
    input: impl BufRead,
) -> Result<()> {
    let api = Api::new(secret)?;
    let mut target: Option<String> = None;
    let (mut batch, mut waiting) = (Vec::new(), 0);
    let (mut indices, mut count) = (0, 0u64);
    for record in input.lines() {
        let record = record?;
        if record.starts_with("{\"dbm_index\":") {
            if let Some(done) = &target {
                bulk(&api, done, &mut batch)?;
                api.send(Method::POST, &format!("{done}/_refresh"), None)?;
            }
            waiting = 0;
            let index: Index = serde_json::from_str(&record)?;
            let next = if source == SEARCH_ALL {
                index.dbm_index.clone()
            } else {
                name.to_owned()
            };
            if !valid_index_name(&next) {
                return Err("invalid index name".into());
            }
            create(&api, &next, &index)?;
            target = Some(next);
            indices += 1;
            continue;
        }
        let current = target.as_deref().ok_or("this is not an index dump")?;
        let document: Document = serde_json::from_str(&record)?;
        let mut action = json!({ "_id": document.id });
        if let Some(routing) = document.routing {
            action["routing"] = routing.into();
        }
        writeln!(batch, "{}", json!({ "index": action }))?;
        writeln!(batch, "{}", document.source.get())?;
        (waiting, count) = (waiting + 1, count + 1);
        if waiting >= PAGE || batch.len() >= BULK_BYTES {
            bulk(&api, current, &mut batch)?;
            waiting = 0;
        }
    }
    if let Some(done) = &target {
        bulk(&api, done, &mut batch)?;
        api.send(Method::POST, &format!("{done}/_refresh"), None)?;
    }
    eprintln!("{indices} indices and {count} documents restored");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_stay_on_one_line_and_keep_their_numbers() {
        let source = RawValue::from_string(
            "{\n \"n\": 12345678901234567890.5,\r\n \"s\": \"a\\nb\" }".into(),
        )
        .unwrap();
        let line = one_line(source).unwrap();
        assert!(!line.get().contains(['\n', '\r']));
        assert!(line.get().contains("12345678901234567890.5"));
        assert!(line.get().contains("a\\nb"));
    }

    #[test]
    fn settings_the_server_owns_are_not_sent_back() {
        let found = json!({ "shop": { "settings": { "index": { "uuid": "x", "number_of_shards": "1" } }, "mappings": {}, "aliases": {} } });
        let index = definition("shop", found);
        assert_eq!(
            index.settings,
            json!({ "index": { "number_of_shards": "1" } })
        );
    }
}
