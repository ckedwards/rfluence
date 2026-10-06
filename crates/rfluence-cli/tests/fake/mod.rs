//! A small in-memory Confluence for tests that need state across requests (`upload --config`
//! creates pages, then reads them back): pages, folders, labels and content properties in one
//! space, served over HTTP on a local port.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

pub const SPACE_ID: &str = "9";
pub const SPACE_KEY: &str = "ENG";
pub const HOMEPAGE: &str = "100";

#[derive(Debug, Clone)]
pub struct Content {
    pub id: String,
    /// `page` or `folder`.
    pub kind: &'static str,
    pub title: String,
    pub parent: Option<String>,
    pub version: u64,
    /// ADF, as a JSON string.
    pub body: String,
    pub labels: Vec<String>,
    /// key -> (property ID, version, value)
    pub properties: BTreeMap<String, (String, u64, Value)>,
    /// Order among siblings (new content goes last).
    pub position: i64,
}

#[derive(Default)]
pub struct State {
    next_id: u64,
    pub content: BTreeMap<String, Content>,
    /// Every request: `METHOD /path?query`.
    pub log: Vec<String>,
    /// Trashed content, by ID.
    pub trashed: BTreeMap<String, Content>,
    /// Requests to answer with an error instead.
    pub failures: Vec<Failure>,
}

/// Answer matching requests with an error: `skip` of them are served normally first, then
/// `times` fail.
#[derive(Debug, Clone)]
pub struct Failure {
    pub method: &'static str,
    /// Part of the path (and query).
    pub contains: String,
    pub status: u16,
    pub retry_after: Option<u64>,
    pub skip: usize,
    pub times: usize,
}

impl Failure {
    pub fn new(method: &'static str, contains: &str, status: u16) -> Failure {
        Failure { method, contains: contains.to_string(), status, retry_after: None, skip: 0, times: usize::MAX }
    }
}

impl State {
    fn new_id(&mut self) -> String {
        self.next_id += 1;
        (1000 + self.next_id).to_string()
    }

    /// Add a page (or folder) and return its ID.
    pub fn add(&mut self, kind: &'static str, title: &str, parent: Option<&str>) -> String {
        let id = self.new_id();
        self.add_with_id(&id, kind, title, parent);
        id
    }

    pub fn add_with_id(&mut self, id: &str, kind: &'static str, title: &str, parent: Option<&str>) {
        let body = r#"{"type":"doc","version":1,"content":[]}"#.to_string();
        let position = self.last_position() + 10;
        let c = Content { id: id.into(), kind, title: title.into(), parent: parent.map(str::to_string), version: 1, body, labels: vec![], properties: BTreeMap::new(), position };
        self.content.insert(id.into(), c);
    }

    fn last_position(&self) -> i64 {
        self.content.values().map(|c| c.position).max().unwrap_or(0)
    }

    /// The titles of a page's or folder's children, in order.
    pub fn children(&self, parent: &str) -> Vec<String> {
        let mut kids: Vec<&Content> = self.content.values().filter(|c| c.parent.as_deref() == Some(parent)).collect();
        kids.sort_by_key(|c| c.position);
        kids.iter().map(|c| c.title.clone()).collect()
    }

    /// Put `id` right before or after `target`, among `target`'s siblings.
    pub fn move_next_to(&mut self, id: &str, after: bool, target: &str) {
        let parent = self.content[target].parent.clone();
        let mut siblings: Vec<String> = self.content.values().filter(|c| c.parent == parent && c.id != id).map(|c| c.id.clone()).collect();
        siblings.sort_by_key(|s| self.content[s].position);
        let at = siblings.iter().position(|s| s == target).unwrap() + usize::from(after);
        siblings.insert(at, id.to_string());
        self.content.get_mut(id).unwrap().parent = parent;
        for (i, s) in siblings.iter().enumerate() {
            self.content.get_mut(s).unwrap().position = (i as i64 + 1) * 10;
        }
    }

    pub fn by_title(&self, kind: &str, title: &str) -> Option<&Content> {
        self.content.values().find(|c| c.kind == kind && c.title == title)
    }

    /// Requests other than reads.
    pub fn writes(&self) -> Vec<&String> {
        self.log.iter().filter(|l| !l.starts_with("GET ")).collect()
    }
}

pub struct Fake {
    pub url: String,
    pub state: Arc<Mutex<State>>,
}

impl Fake {
    /// A site with space ENG, whose homepage is page 100 ("Home").
    pub fn start() -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State::default()));
        state.lock().unwrap().add_with_id(HOMEPAGE, "page", "Home", None);
        let shared = state.clone();
        let base = url.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let state = shared.clone();
                let base = base.clone();
                std::thread::spawn(move || {
                    let _ = serve(stream, &state, &base);
                });
            }
        });
        Fake { url, state }
    }

    pub fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }
}

fn serve(stream: std::net::TcpStream, state: &Mutex<State>, base: &str) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
    let mut length = 0;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        if header.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = header.split_once(':')
            && k.eq_ignore_ascii_case("content-length") {
            length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let (status, response, retry_after) = {
        let mut state = state.lock().unwrap();
        let failure = state.failures.iter_mut().find(|f| f.method == method && target.contains(&f.contains)).and_then(|f| {
            if f.skip > 0 {
                f.skip -= 1;
                None
            } else if f.times > 0 {
                f.times -= 1;
                Some((f.status, f.retry_after))
            } else {
                None
            }
        });
        match failure {
            Some((status, retry_after)) => {
                state.log.push(format!("{method} {target} -> {status}"));
                (status, json!({ "message": format!("injected HTTP {status}") }), retry_after)
            }
            None => {
                let (status, response) = handle(&mut state, &method, &target, &body, base);
                (status, response, None)
            }
        }
    };
    let text = if status == 204 { String::new() } else { response.to_string() };
    let header = retry_after.map(|s| format!("Retry-After: {s}\r\n")).unwrap_or_default();
    let mut stream = stream;
    write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{header}Content-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len())?;
    stream.flush()
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| percent_encoding::percent_decode_str(&v.replace('+', " ")).decode_utf8_lossy().into_owned())
    })
}

fn not_found() -> (u16, Value) {
    (404, json!({ "errors": [{ "status": 404, "title": "Not found" }] }))
}

fn page_json(c: &Content, base: &str, with_body: bool) -> Value {
    let mut v = json!({
        "id": c.id, "title": c.title, "parentId": c.parent, "spaceId": SPACE_ID, "status": "current",
        "version": { "number": c.version, "createdAt": "2026-10-06T10:00:00.000Z" },
        "labels": { "results": c.labels.iter().map(|l| json!({ "name": l, "prefix": "global" })).collect::<Vec<_>>(), "meta": { "hasMore": false } },
        "_links": { "webui": format!("/spaces/{SPACE_KEY}/pages/{}", c.id), "base": format!("{base}/wiki") },
    });
    if with_body {
        v["body"] = json!({ "atlas_doc_format": { "representation": "atlas_doc_format", "value": c.body } });
    }
    v
}

fn handle(state: &mut State, method: &str, target: &str, body: &[u8], base: &str) -> (u16, Value) {
    state.log.push(format!("{method} {target}"));
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let body: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match (method, segments.as_slice()) {
        ("GET", ["wiki", "api", "v2", "spaces"]) => {
            let results = if query_param(query, "keys").as_deref() == Some(SPACE_KEY) {
                vec![json!({ "id": SPACE_ID, "key": SPACE_KEY, "homepageId": HOMEPAGE })]
            } else {
                vec![]
            };
            (200, json!({ "results": results, "_links": {} }))
        }
        ("GET", ["wiki", "api", "v2", kind @ ("pages" | "folders"), id]) => match state.content.get(*id) {
            Some(c) if c.kind == &kind[..kind.len() - 1] => (200, page_json(c, base, *kind == "pages")),
            _ => not_found(),
        },
        ("GET", ["wiki", "api", "v2", "pages"]) => (200, json!({ "results": [], "_links": {} })),
        ("POST", ["wiki", "api", "v2", kind @ ("pages" | "folders")]) => {
            let title = body["title"].as_str().unwrap_or_default().to_string();
            let singular = if *kind == "pages" { "page" } else { "folder" };
            if state.by_title(singular, &title).is_some() {
                return (400, json!({ "errors": [{ "status": 400, "title": format!("A {singular} with this title already exists") }] }));
            }
            let id = state.add(singular, &title, body["parentId"].as_str());
            if let Some(value) = body.pointer("/body/value").and_then(Value::as_str) {
                state.content.get_mut(&id).unwrap().body = value.to_string();
            }
            (200, page_json(&state.content[&id], base, false))
        }
        ("PUT", ["wiki", "api", "v2", "pages", id]) => {
            let Some(c) = state.content.get_mut(*id) else { return not_found() };
            let version = body.pointer("/version/number").and_then(Value::as_u64).unwrap_or(0);
            if version != c.version + 1 {
                return (409, json!({ "errors": [{ "status": 409, "title": "Version must be incremented" }] }));
            }
            // Like Confluence: the same body makes no new version.
            let new_body = body.pointer("/body/value").and_then(Value::as_str).unwrap_or_default().to_string();
            let new_title = body["title"].as_str().unwrap_or_default().to_string();
            let new_parent = body["parentId"].as_str().map(str::to_string);
            if new_body != c.body || new_title != c.title || new_parent.as_ref().is_some_and(|p| Some(p) != c.parent.as_ref()) {
                c.version = version;
            }
            c.body = new_body;
            c.title = new_title;
            if new_parent.is_some() && new_parent != c.parent {
                c.parent = new_parent;
                // A moved page goes last under its new parent.
                let last = state.last_position();
                let c = state.content.get_mut(*id).unwrap();
                c.position = last + 10;
                return (200, page_json(c, base, false));
            }
            (200, page_json(c, base, false))
        }
        ("GET", ["wiki", "api", "v2", "pages", _, "attachments"]) => (200, json!({ "results": [], "_links": {} })),
        ("GET", ["wiki", "api", "v2", "pages" | "folders", id, "properties"]) => {
            let Some(c) = state.content.get(*id) else { return not_found() };
            let key = query_param(query, "key");
            let results: Vec<Value> = c
                .properties
                .iter()
                .filter(|(k, _)| key.as_ref().is_none_or(|key| key == *k))
                .map(|(k, (pid, v, value))| json!({ "id": pid, "key": k, "value": value, "version": { "number": v } }))
                .collect();
            (200, json!({ "results": results, "_links": {} }))
        }
        ("POST", ["wiki", "api", "v2", "pages" | "folders", id, "properties"]) => {
            let pid = state.new_id();
            let Some(c) = state.content.get_mut(*id) else { return not_found() };
            let key = body["key"].as_str().unwrap_or_default().to_string();
            c.properties.insert(key, (pid, 1, body["value"].clone()));
            (200, json!({}))
        }
        ("PUT", ["wiki", "api", "v2", "pages" | "folders", id, "properties", _]) => {
            let Some(c) = state.content.get_mut(*id) else { return not_found() };
            let key = body["key"].as_str().unwrap_or_default().to_string();
            let version = body.pointer("/version/number").and_then(Value::as_u64).unwrap_or(0);
            let entry = c.properties.get_mut(&key).expect("property exists");
            *entry = (entry.0.clone(), version, body["value"].clone());
            (200, json!({}))
        }
        ("GET", ["wiki", "api", "v2", "pages" | "folders", id, "direct-children"]) => {
            let results: Vec<Value> = state
                .content
                .values()
                .filter(|c| c.parent.as_deref() == Some(*id))
                .map(|c| (c.position, json!({ "id": c.id, "type": c.kind, "title": c.title, "childPosition": c.position })))
                .collect::<BTreeMap<_, _>>()
                .into_values()
                .collect();
            (200, json!({ "results": results, "_links": {} }))
        }
        ("GET", ["wiki", "rest", "api", "content"]) => {
            let title = query_param(query, "title").unwrap_or_default();
            let results: Vec<Value> = state.by_title("page", &title).map(|c| json!({ "id": c.id })).into_iter().collect();
            (200, json!({ "results": results }))
        }
        ("GET", ["wiki", "rest", "api", "search"]) => {
            // Only the folder lookup: `type = folder and space = "ENG" and title = "..."`.
            let cql = query_param(query, "cql").unwrap_or_default();
            let title = cql.split("title = \"").nth(1).and_then(|t| t.strip_suffix('"')).unwrap_or_default().replace("\\\"", "\"");
            let results: Vec<Value> =
                state.by_title("folder", &title).map(|c| json!({ "content": { "id": c.id, "title": c.title } })).into_iter().collect();
            (200, json!({ "results": results }))
        }
        ("POST", ["wiki", "rest", "api", "content", id, "label"]) => {
            let Some(c) = state.content.get_mut(*id) else { return not_found() };
            for l in body.as_array().into_iter().flatten() {
                let name = l["name"].as_str().unwrap_or_default().to_string();
                if !c.labels.contains(&name) {
                    c.labels.push(name);
                }
            }
            (200, json!({ "results": [] }))
        }
        ("PUT", ["wiki", "rest", "api", "content", id, "move", position @ ("before" | "after"), target]) => {
            if !state.content.contains_key(*id) || !state.content.contains_key(*target) {
                return not_found();
            }
            state.move_next_to(id, *position == "after", target);
            (200, json!({ "pageId": id }))
        }
        ("DELETE", ["wiki", "rest", "api", "content", id, "label"]) => {
            let Some(c) = state.content.get_mut(*id) else { return not_found() };
            let name = query_param(query, "name").unwrap_or_default();
            c.labels.retain(|l| *l != name);
            (204, Value::Null)
        }
        ("DELETE", ["wiki", "api", "v2", "pages" | "folders", id]) => {
            let Some(c) = state.content.remove(*id) else { return not_found() };
            // Like Confluence: the children move up a level.
            for child in state.content.values_mut().filter(|x| x.parent.as_deref() == Some(*id)) {
                child.parent = c.parent.clone();
            }
            state.trashed.insert(id.to_string(), c);
            (204, Value::Null)
        }
        _ => (501, json!({ "message": format!("the fake doesn't serve {method} {path}") })),
    }
}
