//! The built dashboard (`dashboard/dist`, committed) embedded in the binary.
use include_dir::{include_dir, Dir};

static DIST: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../dashboard/dist");

pub struct Asset {
    pub body: &'static [u8],
    pub content_type: &'static str,
}

/// `"/"` is `index.html`. The directory is embedded, so a path can only name an embedded file;
/// `..`, a leading `//` and backslashes are refused anyway.
pub fn get(path: &str) -> Option<Asset> {
    let rel = path.strip_prefix('/')?;
    if rel.starts_with('/') || rel.contains("..") || rel.contains('\\') {
        return None;
    }
    let rel = if rel.is_empty() { "index.html" } else { rel };
    let file = DIST.get_file(rel)?;
    let content_type = match rel.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "json" => "application/json",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    };
    Some(Asset { body: file.contents(), content_type })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_serves_index_html_and_unknown_paths_404() {
        let a = get("/").unwrap();
        assert_eq!(a.content_type, "text/html; charset=utf-8");
        assert!(String::from_utf8_lossy(a.body).contains(r#"id="app""#));
        assert!(get("/nope.js").is_none());
        assert_eq!(get("/main.js").unwrap().content_type, "text/javascript; charset=utf-8");
    }
    #[test]
    fn traversal_is_refused() {
        for p in ["/../Cargo.toml", "//etc/passwd", "/..\\x", "/a/../index.html"] {
            assert!(get(p).is_none(), "{p}");
        }
    }
}
