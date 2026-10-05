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
        let mut resp = self
            .agent
            .get(&url)
            .header("Authorization", &self.authorization)
            .header("Accept", "application/json")
            .call()
            .map_err(|e| Error::Network(format!("{url}: {e}")))?;
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
        let space_key = raw
            .links
            .webui
            .strip_prefix("/spaces/")
            .and_then(|r| r.split('/').next())
            .unwrap_or_default()
            .to_string();
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

/// fileId -> file name, for [`rfluence_convert::FetchContext::attachments`].
pub fn file_names(attachments: &[Attachment]) -> HashMap<String, String> {
    attachments.iter().map(|a| (a.file_id.clone(), a.title.clone())).collect()
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
    fn encodes_query_values() {
        assert_eq!(encode("Ingestion: overview & more"), "Ingestion%3A%20overview%20%26%20more");
    }
}
