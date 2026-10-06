//! The Confluence REST calls rfluence makes.

use std::collections::HashMap;
use std::time::Duration;

use base64::Engine;
use rfluence_convert::PageMeta;
use rfluence_convert::adf::Node;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::auth::Credentials;
use crate::error::{Error, Result};

/// A Confluence site, with one connection pool shared by all requests (and threads).
#[derive(Clone)]
pub struct Client {
    agent: ureq::Agent,
    base_url: String,
    authorization: String,
}

/// A page as fetched: its metadata and body.
#[derive(Debug, Clone)]
pub struct Page {
    pub meta: PageMeta,
    pub adf: Node,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub title: String,
    #[serde(rename = "fileId")]
    pub file_id: String,
    #[serde(rename = "mediaType", default)]
    pub media_type: String,
    #[serde(rename = "fileSize")]
    pub file_size: Option<u64>,
    #[serde(rename = "_links", default)]
    links: AttachmentLinks,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
struct AttachmentLinks {
    download: Option<String>,
}

/// The largest attachment `download` reads (Confluence Cloud's own limit is far below this).
const MAX_DOWNLOAD: u64 = 2 * 1024 * 1024 * 1024;

/// A search result. See design.md, "Commands" > `rfluence search`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SearchResult {
    pub id: String,
    pub title: String,
    pub space_key: String,
    pub space_name: String,
    pub url: String,
    /// `YYYY-MM-DD`.
    pub updated: Option<String>,
    pub labels: Vec<String>,
    /// Plain text, one line.
    pub excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SearchResults {
    /// How many results match in all (may be more than returned).
    pub total: u64,
    pub results: Vec<SearchResult>,
}

/// The CQL for a search: free text, restricted to pages, in any of `spaces` and with all of
/// `labels`.
pub fn search_cql(text: &str, spaces: &[String], labels: &[String]) -> String {
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let mut parts = vec![format!("text ~ {}", quote(text)), "type = page".to_string()];
    match spaces {
        [] => {}
        [one] => parts.push(format!("space = {}", quote(one))),
        many => parts.push(format!("space in ({})", many.iter().map(|s| quote(s)).collect::<Vec<_>>().join(", "))),
    }
    parts.extend(labels.iter().map(|l| format!("label = {}", quote(l))));
    parts.join(" and ")
}

/// A search excerpt as one line of plain text: highlight markers removed, HTML entities
/// decoded, whitespace collapsed, and cut to about `max` characters at a word.
pub fn clean_excerpt(excerpt: &str, max: usize) -> String {
    let text = excerpt.replace("@@@hl@@@", "").replace("@@@endhl@@@", "");
    let text = decode_entities(&text);
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        return one_line;
    }
    let cut: String = one_line.chars().take(max).collect();
    let at_word = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
    format!("{}…", at_word.trim_end_matches([',', '.', ';', ':']))
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let decoded = rest.find(';').filter(|&e| e <= 10).and_then(|e| {
            let c = match &rest[1..e] {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32),
                n if n.starts_with('#') => n[1..].parse().ok().and_then(char::from_u32),
                _ => None,
            }?;
            Some((c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// How a page was named on the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageRef {
    Id(String),
    Title { space_key: String, title: String },
}

/// Parse a page reference: a page ID, a page URL (`/pages/<id>`, `?pageId=<id>`, or a tiny
/// link `/x/<code>`), or `SPACE:Title`.
pub fn parse_page_ref(s: &str) -> Result<PageRef> {
    let s = s.trim();
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(PageRef::Id(s.to_string()));
    }
    if s.starts_with("http://") || s.starts_with("https://") {
        let path = s.split(['#']).next().unwrap_or(s);
        if let Some(rest) = path.split("/pages/").nth(1) {
            let id: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if !id.is_empty() {
                return Ok(PageRef::Id(id));
            }
        }
        if let Some(q) = path.split("pageId=").nth(1) {
            let id: String = q.chars().take_while(char::is_ascii_digit).collect();
            if !id.is_empty() {
                return Ok(PageRef::Id(id));
            }
        }
        if let Some(code) = path.split("/x/").nth(1) {
            let code = code.split(['/', '?']).next().unwrap_or("");
            if let Some(id) = rfluence_convert::links::tiny_link_id(code) {
                return Ok(PageRef::Id(id.to_string()));
            }
        }
        return Err(Error::Invalid(format!("not a Confluence page URL: {s}")));
    }
    if let Some((space, title)) = s.split_once(':') {
        if !space.is_empty() && !title.trim().is_empty() && !space.contains(char::is_whitespace) {
            return Ok(PageRef::Title { space_key: space.to_string(), title: title.trim().to_string() });
        }
    }
    Err(Error::Invalid(format!("not a page ID, page URL or SPACE:Title: {s}")))
}

/// The site a page reference names: the base URL of a page URL (`None` for IDs and
/// `SPACE:Title`).
pub fn page_ref_site(s: &str) -> Option<String> {
    let s = s.trim();
    if !(s.starts_with("http://") || s.starts_with("https://")) {
        return None;
    }
    let (scheme, rest) = s.split_once("://")?;
    Some(format!("{scheme}://{}", rest.split('/').next()?))
}

impl Client {
    pub fn new(creds: &Credentials) -> Client {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            // Read Confluence's error bodies instead of getting a bare status error.
            .http_status_as_error(false)
            .user_agent(concat!("rfluence/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        let basic = base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", creds.email, creds.token));
        Client {
            agent,
            base_url: crate::auth::normalize_base_url(&creds.base_url),
            authorization: format!("Basic {basic}"),
        }
    }

    /// GET a JSON resource under the site (`path` starts with `/wiki/...`).
    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{path}", self.base_url);
        let resp = self
            .agent
            .get(&url)
            .header("Authorization", &self.authorization)
            .header("Accept", "application/json")
            .call();
        read_json(resp, &url, path)
    }

    /// POST or PUT JSON, and read the JSON response.
    fn send_json<T: DeserializeOwned>(&self, method: &str, path: &str, body: &serde_json::Value) -> Result<T> {
        let url = format!("{}{path}", self.base_url);
        let req = match method {
            "PUT" => self.agent.put(&url),
            _ => self.agent.post(&url),
        };
        let resp = req.header("Authorization", &self.authorization).header("Accept", "application/json").send_json(body);
        read_json(resp, &url, path)
    }

    /// POST a file as `multipart/form-data` (v1 attachment uploads), and read the JSON response.
    fn send_file<T: DeserializeOwned>(&self, path: &str, file_name: &str, data: &[u8]) -> Result<T> {
        let url = format!("{}{path}", self.base_url);
        let boundary = format!("rfluence-{:016x}", data.len() as u64 ^ 0x9e37_79b9_7f4a_7c15);
        let name = file_name.replace(['"', '\r', '\n'], "_");
        let mut body = Vec::with_capacity(data.len() + 512);
        body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"minorEdit\"\r\n\r\ntrue\r\n").bytes());
        body.extend(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: {}\r\n\r\n",
                media_type(file_name)
            )
            .bytes(),
        );
        body.extend(data);
        body.extend(format!("\r\n--{boundary}--\r\n").bytes());
        let resp = self
            .agent
            .post(&url)
            .header("Authorization", &self.authorization)
            .header("Accept", "application/json")
            .header("X-Atlassian-Token", "no-check")
            .header("Content-Type", &format!("multipart/form-data; boundary={boundary}"))
            .send(&body[..]);
        read_json(resp, &url, path)
    }

    /// The signed-in user's display name (to check credentials).
    pub fn current_user(&self) -> Result<String> {
        #[derive(Deserialize)]
        struct User {
            #[serde(rename = "displayName")]
            display_name: String,
        }
        Ok(self.get::<User>("/wiki/rest/api/user/current")?.display_name)
    }

    /// A page's body (ADF) and metadata, in one call (plus one per extra page of labels).
    pub fn page(&self, id: &str) -> Result<Page> {
        self.page_at(id, None)
    }

    /// An earlier version of a page.
    pub fn page_version(&self, id: &str, version: u64) -> Result<Page> {
        self.page_at(id, Some(version))
    }

    fn page_at(&self, id: &str, version: Option<u64>) -> Result<Page> {
        let version_query = version.map(|v| format!("&version={v}")).unwrap_or_default();
        let raw: RawPage = self
            .get(&format!("/wiki/api/v2/pages/{id}?body-format=atlas_doc_format&include-labels=true{version_query}"))
            .map_err(|e| page_not_found(e, id))?;
        let mut labels: Vec<String> = Vec::new();
        let mut more = false;
        if let Some(l) = &raw.labels {
            labels.extend(l.results.iter().filter(|l| l.prefix == "global").map(|l| l.name.clone()));
            more = l.meta.as_ref().is_some_and(|m| m.has_more);
        }
        if more {
            labels = self.labels(id)?;
        }
        let space_key = space_key_of(&raw.links.webui);
        let adf: Node = serde_json::from_str(&raw.body.atlas_doc_format.value)
            .map_err(|e| Error::Api { status: 200, message: format!("page {id} body isn't valid ADF: {e}") })?;
        Ok(Page {
            meta: PageMeta {
                url: format!("{}/spaces/{space_key}/pages/{}", raw.links.base, raw.id),
                id: raw.id,
                title: raw.title,
                space_key,
                parent: raw.parent_id,
                version: raw.version.number,
                updated: raw.version.created_at,
                labels,
            },
            adf,
        })
    }

    /// All of a page's global labels (when the page response said there are more).
    fn labels(&self, id: &str) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let mut path = format!("/wiki/api/v2/pages/{id}/labels?limit=250");
        loop {
            let page: Paged<RawLabel> = self.get(&path)?;
            out.extend(page.results.into_iter().filter(|l| l.prefix == "global").map(|l| l.name));
            match page.links.and_then(|l| l.next) {
                Some(next) => path = format!("/wiki{next}"),
                None => return Ok(out),
            }
        }
    }

    /// A page's attachments.
    pub fn attachments(&self, id: &str) -> Result<Vec<Attachment>> {
        let mut out = Vec::new();
        let mut path = format!("/wiki/api/v2/pages/{id}/attachments?limit=250");
        loop {
            let page: Paged<Attachment> = self.get(&path)?;
            out.extend(page.results);
            match page.links.and_then(|l| l.next) {
                Some(next) => path = format!("/wiki{next}"),
                None => return Ok(out),
            }
        }
    }

    /// The titles of pages, by ID: one request per 250 pages. Pages that don't exist or
    /// aren't visible are missing from the result.
    pub fn page_titles(&self, ids: &[String]) -> Result<HashMap<String, String>> {
        #[derive(Deserialize)]
        struct Title {
            id: String,
            title: String,
        }
        let mut titles = HashMap::new();
        for chunk in ids.chunks(250) {
            let mut path = format!("/wiki/api/v2/pages?limit=250&id={}", chunk.join(","));
            loop {
                let page: Paged<Title> = self.get(&path)?;
                titles.extend(page.results.into_iter().map(|t| (t.id, t.title)));
                match page.links.and_then(|l| l.next) {
                    Some(next) => path = format!("/wiki{next}"),
                    None => break,
                }
            }
        }
        Ok(titles)
    }

    /// The bodies of published pages, by ID: one request per 250 pages. Drafts, and pages
    /// that don't exist or aren't visible, are missing from the result.
    pub fn page_bodies(&self, ids: &[String]) -> Result<HashMap<String, Node>> {
        #[derive(Deserialize)]
        struct WithBody {
            id: String,
            body: RawBody,
        }
        let mut bodies = HashMap::new();
        for chunk in ids.chunks(250) {
            let mut path = format!("/wiki/api/v2/pages?limit=250&body-format=atlas_doc_format&id={}", chunk.join(","));
            loop {
                let page: Paged<WithBody> = self.get(&path)?;
                for p in page.results {
                    if let Ok(doc) = serde_json::from_str(&p.body.atlas_doc_format.value) {
                        bodies.insert(p.id, doc);
                    }
                }
                match page.links.and_then(|l| l.next) {
                    Some(next) => path = format!("/wiki{next}"),
                    None => break,
                }
            }
        }
        Ok(bodies)
    }

    /// The content of synced block copies, by `resourceId`, read from their source pages
    /// (one request). Best effort: copies whose source can't be read are missing.
    pub fn synced_copies(&self, ids: &[String]) -> HashMap<String, Vec<Node>> {
        let mut pages: Vec<String> = ids.iter().filter_map(|r| rfluence_convert::synced::parse_copy(r)).map(|(p, _)| p.to_string()).collect();
        pages.sort();
        pages.dedup();
        if pages.is_empty() {
            return HashMap::new();
        }
        let sources = self.page_bodies(&pages).unwrap_or_default();
        rfluence_convert::synced::copy_contents(ids, &sources)
    }

    /// Search with CQL (v1 search: v2 has none). One request, labels included.
    pub fn search(&self, cql: &str, limit: usize) -> Result<SearchResults> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(rename = "totalSize", default)]
            total_size: u64,
            results: Vec<RawResult>,
            #[serde(rename = "_links")]
            links: RawSearchLinks,
        }
        #[derive(Deserialize)]
        struct RawSearchLinks {
            base: String,
        }
        #[derive(Deserialize)]
        struct RawResult {
            content: Option<RawContent>,
            #[serde(default)]
            excerpt: String,
            #[serde(rename = "lastModified")]
            last_modified: Option<String>,
            #[serde(rename = "resultGlobalContainer")]
            container: Option<RawContainer>,
        }
        #[derive(Deserialize)]
        struct RawContent {
            id: String,
            title: String,
            metadata: Option<RawMetadata>,
        }
        #[derive(Deserialize)]
        struct RawMetadata {
            labels: Option<RawLabels>,
        }
        #[derive(Deserialize)]
        struct RawContainer {
            title: String,
            #[serde(rename = "displayUrl", default)]
            display_url: String,
        }
        let query = format!("cql={}&limit={limit}&expand=content.metadata.labels", encode(cql));
        let raw: Raw = self.get(&format!("/wiki/rest/api/search?{query}")).map_err(|e| match e {
            Error::Api { status: 400, message } => {
                // "com.atlassian...BadRequestException: Could not parse cql : ..." -> the reason.
                let reason = message.split_once("Exception: ").map_or(message.as_str(), |(_, r)| r).trim();
                Error::Invalid(format!("invalid CQL query: {reason} (query: {cql})"))
            }
            e => e,
        })?;
        let results = raw
            .results
            .into_iter()
            .filter_map(|r| {
                let content = r.content?;
                let container = r.container;
                let space_key = container
                    .as_ref()
                    .and_then(|c| c.display_url.strip_prefix("/spaces/"))
                    .unwrap_or_default()
                    .to_string();
                let labels = content
                    .metadata
                    .and_then(|m| m.labels)
                    .map(|l| l.results.into_iter().filter(|l| l.prefix == "global").map(|l| l.name).collect())
                    .unwrap_or_default();
                Some(SearchResult {
                    url: format!("{}/spaces/{space_key}/pages/{}", raw.links.base, content.id),
                    id: content.id,
                    title: content.title,
                    space_name: container.map(|c| c.title).unwrap_or_default(),
                    space_key,
                    updated: r.last_modified.map(|d| d.chars().take(10).collect()),
                    labels,
                    excerpt: clean_excerpt(&r.excerpt, 220),
                })
            })
            .collect();
        Ok(SearchResults { total: raw.total_size, results })
    }

    /// An attachment's content.
    pub fn download(&self, attachment: &Attachment) -> Result<Vec<u8>> {
        let link = attachment
            .links
            .download
            .as_deref()
            .ok_or_else(|| Error::NotFound(format!("attachment {} has no download link", attachment.title)))?;
        let url = format!("{}/wiki{link}", self.base_url);
        let mut resp = self
            .agent
            .get(&url)
            .header("Authorization", &self.authorization)
            .call()
            .map_err(|e| Error::Network(format!("{url}: {e}")))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(match status {
                401 | 403 => Error::Auth(format!("HTTP {status} downloading {}", attachment.title)),
                404 => Error::NotFound(format!("attachment {} not found", attachment.title)),
                _ => Error::Api { status, message: format!("downloading {}", attachment.title) },
            });
        }
        resp.body_mut()
            .with_config()
            .limit(MAX_DOWNLOAD)
            .read_to_vec()
            .map_err(|e| Error::Network(format!("{url}: {e}")))
    }

    /// The ID of the page titled `title` in space `space_key` (titles are unique per space).
    pub fn page_id_by_title(&self, space_key: &str, title: &str) -> Result<String> {
        #[derive(Deserialize)]
        struct Content {
            id: String,
        }
        let query = format!("type=page&limit=1&spaceKey={}&title={}", encode(space_key), encode(title));
        let found: Paged<Content> = self.get(&format!("/wiki/rest/api/content?{query}"))?;
        found
            .results
            .into_iter()
            .next()
            .map(|c| c.id)
            .ok_or_else(|| Error::NotFound(format!("no page titled {title:?} in space {space_key}")))
    }

    pub fn resolve(&self, page: &PageRef) -> Result<String> {
        match page {
            PageRef::Id(id) => Ok(id.clone()),
            PageRef::Title { space_key, title } => self.page_id_by_title(space_key, title),
        }
    }

    /// A page and its attachments, requested at the same time: the attachments name the
    /// page's image files, so fetching them in parallel keeps `rfluence fetch` to one round trip
    /// once the page ID is known.
    pub fn page_with_attachments(&self, id: &str) -> Result<(Page, Vec<Attachment>)> {
        std::thread::scope(|s| {
            let attachments = s.spawn(|| self.attachments(id));
            let page = self.page(id)?;
            let attachments = attachments.join().expect("attachments thread doesn't panic")?;
            Ok((page, attachments))
        })
    }
}

/// What a page update sent back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Updated {
    pub version: u64,
    pub title: String,
    pub parent: Option<String>,
}

/// Pages and folders: content IDs are shared, so an ID is one or the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Page,
    Folder,
}

impl Kind {
    fn path(self) -> &'static str {
        match self {
            Kind::Page => "pages",
            Kind::Folder => "folders",
        }
    }
}

/// A page or folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Content {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    pub parent_id: Option<String>,
    pub space_id: Option<String>,
}

/// A page or folder in its parent's list of children.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Child {
    pub id: String,
    /// `page` or `folder`.
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    /// Opaque and sparse: only for comparing.
    #[serde(rename = "childPosition", default)]
    pub position: i64,
}

/// A space: what creating a page in it needs.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Space {
    pub id: String,
    pub key: String,
    #[serde(rename = "homepageId")]
    pub homepage_id: Option<String>,
}

/// A Forge macro installed on the site.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct InstalledMacro {
    #[serde(rename = "appId")]
    pub app_id: String,
    #[serde(rename = "environmentId")]
    pub environment_id: String,
    pub key: String,
}

/// A content property (`rfluence` on managed pages; design.md, "Renames and deletions").
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Property {
    pub id: String,
    pub key: String,
    pub value: serde_json::Value,
    pub version: PropertyVersion,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PropertyVersion {
    pub number: u64,
}

impl Client {
    /// Replace a page's title and body: the new version is `version`, which must be one more
    /// than the current one (Confluence answers 409 otherwise, reported as a conflict).
    pub fn update_page(&self, id: &str, title: &str, adf: &Node, version: u64) -> Result<Updated> {
        self.put_page(id, title, Some(adf), version, None)
    }

    /// Update a page's body (if given) and move it under `parent` (if given), as one new
    /// version. Without a body, Confluence keeps the current one (design.md, "Page hierarchy").
    pub fn put_page(&self, id: &str, title: &str, adf: Option<&Node>, version: u64, parent: Option<&str>) -> Result<Updated> {
        #[derive(Deserialize)]
        struct Raw {
            title: String,
            #[serde(rename = "parentId")]
            parent_id: Option<String>,
            version: RawVersion,
        }
        let mut body = serde_json::json!({
            "id": id,
            "status": "current",
            "title": title,
            "version": { "number": version, "message": "Uploaded with rfluence" },
        });
        if let Some(adf) = adf {
            body["body"] = serde_json::json!({ "representation": "atlas_doc_format", "value": serde_json::to_string(adf).expect("ADF serializes") });
        }
        if let Some(parent) = parent {
            body["parentId"] = parent.into();
        }
        let raw: Raw = self.send_json("PUT", &format!("/wiki/api/v2/pages/{id}"), &body).map_err(|e| match e {
            Error::Api { status: 409, message } => Error::Conflict(format!("page {id} was changed while uploading: {message}")),
            e => page_not_found(e, id),
        })?;
        Ok(Updated { version: raw.version.number, title: raw.title, parent: raw.parent_id })
    }

    /// The space with this key.
    pub fn space(&self, key: &str) -> Result<Space> {
        let found: Paged<Space> = self.get(&format!("/wiki/api/v2/spaces?keys={}", encode(key)))?;
        found
            .results
            .into_iter()
            .find(|s| s.key == key)
            .ok_or_else(|| Error::NotFound(format!("no space with key {key} (or not visible to this account)")))
    }

    /// Create a page (version 1) under `parent` (a page or folder ID) in a space.
    pub fn create_page(&self, space_id: &str, parent: &str, title: &str, adf: &Node) -> Result<PageMeta> {
        #[derive(Deserialize)]
        struct Raw {
            id: String,
            title: String,
            #[serde(rename = "parentId")]
            parent_id: Option<String>,
            version: RawVersion,
            #[serde(rename = "_links")]
            links: RawLinks,
        }
        let body = serde_json::json!({
            "spaceId": space_id,
            "status": "current",
            "title": title,
            "parentId": parent,
            "body": { "representation": "atlas_doc_format", "value": serde_json::to_string(adf).expect("ADF serializes") },
        });
        let raw: Raw = self.send_json("POST", "/wiki/api/v2/pages", &body)?;
        let space_key = space_key_of(&raw.links.webui);
        Ok(PageMeta {
            url: format!("{}/spaces/{space_key}/pages/{}", raw.links.base, raw.id),
            id: raw.id,
            title: raw.title,
            space_key,
            parent: raw.parent_id,
            version: raw.version.number,
            updated: raw.version.created_at,
            labels: Vec::new(),
        })
    }

    /// The Forge macros installed on the site, with their app and environment IDs: the query
    /// the editor uses for its macro menu (design.md, "Mermaid diagrams on sites without
    /// merfluence"). Two requests: the site's cloud ID, then the GraphQL query.
    pub fn installed_macros(&self) -> Result<Vec<InstalledMacro>> {
        #[derive(Deserialize)]
        struct Tenant {
            #[serde(rename = "cloudId")]
            cloud_id: String,
        }
        #[derive(Deserialize)]
        struct Response {
            data: Option<Data>,
            #[serde(default)]
            errors: Vec<serde_json::Value>,
        }
        #[derive(Deserialize)]
        struct Data {
            #[serde(rename = "extensionContexts")]
            contexts: Vec<Context>,
        }
        #[derive(Deserialize)]
        struct Context {
            #[serde(rename = "extensionsByType")]
            extensions: Vec<InstalledMacro>,
        }
        let tenant: Tenant = self.get("/_edge/tenant_info")?;
        let query = "query rfluence_macros($contextIds: [ID!]!, $type: String!) { extensionContexts(contextIds: $contextIds) { extensionsByType(type: $type) { appId environmentId key } } }";
        let body = serde_json::json!({
            "operationName": "rfluence_macros",
            "query": query,
            "variables": { "contextIds": [format!("ari:cloud:confluence::site/{}", tenant.cloud_id)], "type": "xen:macro" },
        });
        let response: Response = self.send_json("POST", "/gateway/api/graphql", &body)?;
        if let Some(error) = response.errors.first() {
            let message = error.get("message").and_then(|m| m.as_str()).unwrap_or("GraphQL error").to_string();
            return Err(Error::Api { status: 200, message });
        }
        Ok(response.data.map(|d| d.contexts.into_iter().flat_map(|c| c.extensions).collect()).unwrap_or_default())
    }

    /// Put a page or folder right before or after a sibling (v1; works for folders too).
    /// Doesn't create a version.
    pub fn move_next_to(&self, id: &str, after: bool, target: &str) -> Result<()> {
        let position = if after { "after" } else { "before" };
        let url = format!("{}/wiki/rest/api/content/{id}/move/{position}/{target}", self.base_url);
        let resp = self.agent.put(&url).header("Authorization", &self.authorization).header("Accept", "application/json").send_empty();
        let _: serde_json::Value = read_json(resp, &url, &format!("/wiki/rest/api/content/{id}/move"))?;
        Ok(())
    }

    /// Move a page to the trash (it can be restored from there).
    pub fn trash_page(&self, id: &str) -> Result<()> {
        self.trash(Kind::Page, id)
    }

    /// Move a page or folder to the trash. A folder's or page's children move up a level.
    pub fn trash(&self, kind: Kind, id: &str) -> Result<()> {
        let url = format!("{}/wiki/api/v2/{}/{id}", self.base_url, kind.path());
        let resp = self.agent.delete(&url).header("Authorization", &self.authorization).call();
        let mut resp = resp.map_err(|e| Error::Network(format!("{url}: {e}")))?;
        match resp.status().as_u16() {
            200..=299 => Ok(()),
            status => {
                let body = resp.body_mut().read_to_string().unwrap_or_default();
                let message = error_message(&body).unwrap_or(body);
                Err(match status {
                    401 | 403 => Error::Auth(format!("HTTP {status} trashing page {id}: {message}")),
                    404 => Error::NotFound(format!("page {id} not found")),
                    _ => Error::Api { status, message },
                })
            }
        }
    }

    /// Attach a new file to a page (v1: v2 can't upload). Doesn't create a page version.
    pub fn upload_attachment(&self, page_id: &str, file_name: &str, data: &[u8]) -> Result<Attachment> {
        #[derive(Deserialize)]
        struct Created {
            results: Vec<V1Attachment>,
        }
        let created: Created = self.send_file(&format!("/wiki/rest/api/content/{page_id}/child/attachment"), file_name, data)?;
        created
            .results
            .into_iter()
            .next()
            .map(V1Attachment::into_attachment)
            .ok_or_else(|| Error::Api { status: 200, message: format!("uploading {file_name}: no attachment in the response") })
    }

    /// Upload a new version of an attachment. The new version gets a new `fileId`.
    pub fn update_attachment(&self, page_id: &str, attachment: &Attachment, data: &[u8]) -> Result<Attachment> {
        let path = format!("/wiki/rest/api/content/{page_id}/child/attachment/{}/data", attachment.id);
        let updated: V1Attachment = self.send_file(&path, &attachment.title, data)?;
        Ok(updated.into_attachment())
    }

    /// Add global labels to a page (v1: v2 can only read labels). Labels must be normalized
    /// (see `rfluence_convert::labels`).
    pub fn add_labels(&self, page_id: &str, labels: &[String]) -> Result<()> {
        if labels.is_empty() {
            return Ok(());
        }
        let body: Vec<_> = labels.iter().map(|l| serde_json::json!({ "prefix": "global", "name": l })).collect();
        let _: serde_json::Value = self.send_json("POST", &format!("/wiki/rest/api/content/{page_id}/label"), &body.into())?;
        Ok(())
    }

    /// A page's content property, if set.
    pub fn property(&self, page_id: &str, key: &str) -> Result<Option<Property>> {
        self.property_of(Kind::Page, page_id, key)
    }

    /// Create or update a page's content property (`existing` from [`Client::property`]).
    pub fn set_property(&self, page_id: &str, key: &str, value: serde_json::Value, existing: Option<&Property>) -> Result<()> {
        self.set_property_of(Kind::Page, page_id, key, value, existing)
    }

    /// A page's or folder's content property, if set.
    pub fn property_of(&self, kind: Kind, id: &str, key: &str) -> Result<Option<Property>> {
        let found: Paged<Property> = self.get(&format!("/wiki/api/v2/{}/{id}/properties?key={}", kind.path(), encode(key)))?;
        Ok(found.results.into_iter().find(|p| p.key == key))
    }

    /// Create or update a page's or folder's content property.
    pub fn set_property_of(&self, kind: Kind, id: &str, key: &str, value: serde_json::Value, existing: Option<&Property>) -> Result<()> {
        let base = format!("/wiki/api/v2/{}/{id}/properties", kind.path());
        let _: serde_json::Value = match existing {
            Some(p) => self.send_json(
                "PUT",
                &format!("{base}/{}", p.id),
                &serde_json::json!({ "key": key, "value": value, "version": { "number": p.version.number + 1 } }),
            )?,
            None => self.send_json("POST", &base, &serde_json::json!({ "key": key, "value": value }))?,
        };
        Ok(())
    }

    /// A page or folder: what it is, its title, where it is.
    pub fn content(&self, id: &str) -> Result<Content> {
        #[derive(Deserialize)]
        struct Raw {
            id: String,
            title: String,
            #[serde(rename = "parentId")]
            parent_id: Option<String>,
            #[serde(rename = "spaceId")]
            space_id: Option<String>,
        }
        let into = |raw: Raw, kind| Content { id: raw.id, kind, title: raw.title, parent_id: raw.parent_id, space_id: raw.space_id };
        match self.get::<Raw>(&format!("/wiki/api/v2/pages/{id}")) {
            Ok(raw) => Ok(into(raw, Kind::Page)),
            Err(Error::NotFound(_)) => match self.get::<Raw>(&format!("/wiki/api/v2/folders/{id}")) {
                Ok(raw) => Ok(into(raw, Kind::Folder)),
                Err(Error::NotFound(_)) => Err(Error::NotFound(format!("no page or folder {id} (or not visible to this account)"))),
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        }
    }

    /// The pages and folders directly under a page or folder, in their order.
    pub fn children(&self, kind: Kind, id: &str) -> Result<Vec<Child>> {
        let mut out = Vec::new();
        let mut path = format!("/wiki/api/v2/{}/{id}/direct-children?limit=250", kind.path());
        loop {
            let page: Paged<Child> = self.get(&path)?;
            out.extend(page.results.into_iter().filter(|c| matches!(c.kind.as_str(), "page" | "folder")));
            match page.links.and_then(|l| l.next) {
                Some(next) => path = format!("/wiki{next}"),
                None => return Ok(out),
            }
        }
    }

    /// The folders titled `title` in a space (folder titles are unique per space, so at most
    /// one, unless it's in the trash). CQL: the v1 content API can't list folders.
    pub fn folders_titled(&self, space_key: &str, title: &str) -> Result<Vec<String>> {
        let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
        let cql = format!("type = folder and space = {} and title = {}", quote(space_key), quote(title));
        #[derive(Deserialize)]
        struct Raw {
            results: Vec<RawResult>,
        }
        #[derive(Deserialize)]
        struct RawResult {
            content: Option<RawContent>,
        }
        #[derive(Deserialize)]
        struct RawContent {
            id: String,
            title: String,
        }
        let raw: Raw = self.get(&format!("/wiki/rest/api/search?cql={}&limit=10", encode(&cql)))?;
        Ok(raw.results.into_iter().filter_map(|r| r.content).filter(|c| c.title == title).map(|c| c.id).collect())
    }

    /// Create a folder (under a page or folder) in a space.
    pub fn create_folder(&self, space_id: &str, parent_id: &str, title: &str) -> Result<String> {
        #[derive(Deserialize)]
        struct Raw {
            id: String,
        }
        let body = serde_json::json!({ "spaceId": space_id, "title": title, "parentId": parent_id });
        let raw: Raw = self.send_json("POST", "/wiki/api/v2/folders", &body)?;
        Ok(raw.id)
    }
}

/// An attachment in a v1 response.
#[derive(Deserialize)]
struct V1Attachment {
    id: String,
    title: String,
    extensions: V1Extensions,
    #[serde(rename = "_links", default)]
    links: AttachmentLinks,
}

#[derive(Deserialize)]
struct V1Extensions {
    #[serde(rename = "fileId")]
    file_id: String,
    #[serde(rename = "mediaType", default)]
    media_type: String,
    #[serde(rename = "fileSize")]
    file_size: Option<u64>,
}

impl V1Attachment {
    fn into_attachment(self) -> Attachment {
        Attachment {
            id: self.id,
            title: self.title,
            file_id: self.extensions.file_id,
            media_type: self.extensions.media_type,
            file_size: self.extensions.file_size,
            links: self.links,
        }
    }
}

/// A response as JSON, or the error it reports.
fn read_json<T: DeserializeOwned>(resp: std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error>, url: &str, path: &str) -> Result<T> {
    let mut resp = resp.map_err(|e| Error::Network(format!("{url}: {e}")))?;
    let status = resp.status().as_u16();
    if (200..300).contains(&status) {
        return resp.body_mut().read_json().map_err(|e| Error::Network(format!("{url}: reading the response: {e}")));
    }
    let body = resp.body_mut().read_to_string().unwrap_or_default();
    let message = error_message(&body).unwrap_or_else(|| body.chars().take(300).collect());
    Err(match status {
        401 | 403 => Error::Auth(format!("HTTP {status} from {url}: {message}")),
        404 => Error::NotFound(format!("not found: {path} ({message})")),
        _ => Error::Api { status, message },
    })
}

/// The media type for an attachment, by file extension.
fn media_type(file_name: &str) -> &'static str {
    let ext = file_name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "tif" | "tiff" => "image/tiff",
        "avif" => "image/avif",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// fileId -> file name, for [`rfluence_convert::FetchContext::attachments`].
pub fn file_names(attachments: &[Attachment]) -> HashMap<String, String> {
    attachments.iter().map(|a| (a.file_id.clone(), a.title.clone())).collect()
}

/// The space key in a page's `webui` link (`/spaces/<KEY>/pages/...`).
fn space_key_of(webui: &str) -> String {
    webui.strip_prefix("/spaces/").and_then(|r| r.split('/').next()).unwrap_or_default().to_string()
}

fn page_not_found(e: Error, id: &str) -> Error {
    match e {
        Error::NotFound(_) => Error::NotFound(format!("page {id} not found (or not visible to this account)")),
        e => e,
    }
}

/// The message in a Confluence error response (v1 `message`, v2 `errors[].title`).
fn error_message(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v.get("message")
        .and_then(|m| m.as_str())
        .map(str::to_string)
        .or_else(|| v.pointer("/errors/0/title").and_then(|t| t.as_str()).map(str::to_string))
}

/// Percent-encode a query value.
fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[derive(Deserialize)]
struct Paged<T> {
    results: Vec<T>,
    #[serde(rename = "_links")]
    links: Option<NextLink>,
}

#[derive(Deserialize)]
struct NextLink {
    next: Option<String>,
}

#[derive(Deserialize)]
struct RawPage {
    id: String,
    title: String,
    #[serde(rename = "parentId")]
    parent_id: Option<String>,
    version: RawVersion,
    labels: Option<RawLabels>,
    body: RawBody,
    #[serde(rename = "_links")]
    links: RawLinks,
}

#[derive(Deserialize)]
struct RawVersion {
    number: u64,
    #[serde(rename = "createdAt")]
    created_at: Option<String>,
}

#[derive(Deserialize)]
struct RawLabels {
    results: Vec<RawLabel>,
    meta: Option<RawMeta>,
}

#[derive(Deserialize)]
struct RawLabel {
    name: String,
    #[serde(default)]
    prefix: String,
}

#[derive(Deserialize)]
struct RawMeta {
    #[serde(rename = "hasMore", default)]
    has_more: bool,
}

#[derive(Deserialize)]
struct RawBody {
    atlas_doc_format: RawValue,
}

#[derive(Deserialize)]
struct RawValue {
    value: String,
}

#[derive(Deserialize)]
struct RawLinks {
    webui: String,
    base: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_page_references() {
        let id = |s: &str| parse_page_ref(s).unwrap();
        assert_eq!(id("295349"), PageRef::Id("295349".into()));
        assert_eq!(id("https://x.atlassian.net/wiki/spaces/ENG/pages/123/Some+Title#Install"), PageRef::Id("123".into()));
        assert_eq!(id("https://x.atlassian.net/wiki/pages/viewpage.action?pageId=456"), PageRef::Id("456".into()));
        assert_eq!(id("https://x.atlassian.net/wiki/x/tYEE"), PageRef::Id("295349".into()));
        assert_eq!(
            id("ENG:Ingestion: overview"),
            PageRef::Title { space_key: "ENG".into(), title: "Ingestion: overview".into() }
        );
        assert!(parse_page_ref("https://x.atlassian.net/wiki/spaces/ENG/overview").is_err());
        assert!(parse_page_ref("just words").is_err());
        assert_eq!(
            page_ref_site("https://other.atlassian.net/wiki/spaces/ENG/pages/1/T").as_deref(),
            Some("https://other.atlassian.net")
        );
        assert_eq!(page_ref_site("123"), None);
    }

    #[test]
    fn builds_search_cql() {
        assert_eq!(search_cql("rate limit", &[], &[]), r#"text ~ "rate limit" and type = page"#);
        assert_eq!(
            search_cql(r#"say "hi""#, &["ENG".into(), "OPS".into()], &["api".into()]),
            r#"text ~ "say \"hi\"" and type = page and space in ("ENG", "OPS") and label = "api""#
        );
    }

    #[test]
    fn cleans_excerpts() {
        assert_eq!(clean_excerpt("We&#39;ve @@@hl@@@added@@@endhl@@@\n  some -&gt; things", 200), "We've added some -> things");
        assert_eq!(clean_excerpt("one two three four", 12), "one two…");
        assert_eq!(clean_excerpt("a & b", 200), "a & b");
    }

    #[test]
    fn encodes_query_values() {
        assert_eq!(encode("Ingestion: overview & more"), "Ingestion%3A%20overview%20%26%20more");
    }
}
