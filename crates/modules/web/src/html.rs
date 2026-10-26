use html5ever::serialize::{SerializeOpts, TraversalScope};
use html5ever::tendril::TendrilSink;
use std::cell::RefCell;
use std::rc::Rc;

use markup5ever_rcdom::{Handle, Node, NodeData, RcDom, SerializableHandle};
use url::Url;

pub(crate) fn parse(source: &str) -> Handle {
    html5ever::parse_document(RcDom::default(), Default::default())
        .one(source)
        .document
}

// Walk in document order without recursing on an untrusted page's nesting.
pub(crate) fn descendants(root: &Handle) -> impl Iterator<Item = Handle> {
    let mut pending = vec![root.clone()];
    std::iter::from_fn(move || {
        let node = pending.pop()?;
        pending.extend(node.children.borrow().iter().rev().cloned());
        Some(node)
    })
}

pub(crate) fn is_element(node: &Handle, tag: &str) -> bool {
    matches!(&node.data, NodeData::Element { name, .. } if name.local.as_ref() == tag)
}

pub(crate) fn attribute(node: &Handle, key: &str) -> Option<String> {
    let NodeData::Element { attrs, .. } = &node.data else {
        return None;
    };
    attrs
        .borrow()
        .iter()
        .find(|attr| attr.name.local.as_ref() == key)
        .map(|attr| attr.value.to_string())
}

pub(crate) fn has_class(node: &Handle, class: &str) -> bool {
    attribute(node, "class")
        .is_some_and(|value| value.split_whitespace().any(|token| token == class))
}

pub(crate) fn text(node: &Handle) -> String {
    descendants(node)
        .filter_map(|node| match &node.data {
            NodeData::Text { contents } => Some(contents.borrow().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Page chrome, controls and embedded media: never the main content.
const CHROME: &[&str] = &[
    "script", "style", "nav", "footer", "aside", "noscript", "svg", "button", "iframe", "template",
    "dialog", "select", "input", "textarea", "canvas", "object", "embed", "video", "audio",
];

/// Roles that mark chrome, as Mozilla Readability treats them.
const CHROME_ROLES: &[&str] = &[
    "navigation",
    "banner",
    "contentinfo",
    "complementary",
    "search",
    "menu",
    "menubar",
    "dialog",
    "alertdialog",
];

/// Inside these a `header` heads its section; elsewhere it is the page's.
const SECTIONING: &[&str] = &["article", "aside", "main", "nav", "section"];

fn tag(node: &Handle) -> Option<&str> {
    match &node.data {
        NodeData::Element { name, .. } => Some(name.local.as_ref()),
        _ => None,
    }
}

fn invisible(node: &Handle) -> bool {
    attribute(node, "hidden").is_some()
        || attribute(node, "aria-hidden").is_some_and(|value| value.trim() == "true")
        || attribute(node, "style").is_some_and(|style| {
            style.split(';').any(|rule| {
                let rule = rule.split_whitespace().collect::<String>();
                rule.to_ascii_lowercase().starts_with("display:none")
            })
        })
}

fn is_chrome(node: &Handle, in_section: bool) -> bool {
    let Some(tag) = tag(node) else {
        return false;
    };
    if matches!(tag, "html" | "body") {
        return false;
    }
    CHROME.contains(&tag)
        // A post's own heading block often sits in an unsectioned header.
        || (tag == "header" && !in_section && !descendants(node).any(|node| is_element(&node, "h1")))
        || attribute(node, "role").is_some_and(|role| {
            role.split_whitespace()
                .any(|role| CHROME_ROLES.contains(&role.to_ascii_lowercase().as_str()))
        })
        || invisible(node)
}

fn text_node(text: &str, parent: &Handle) -> Handle {
    let node = Node::new(NodeData::Text {
        contents: RefCell::new(text.into()),
    });
    node.parent.set(Some(Rc::downgrade(parent)));
    node
}

fn clean(document: &Handle, base: &Url) {
    let mut pending = vec![(document.clone(), false)];
    while let Some((node, in_section)) = pending.pop() {
        let in_section = in_section || tag(&node).is_some_and(|tag| SECTIONING.contains(&tag));
        let children = std::mem::take(&mut *node.children.borrow_mut());
        let mut kept = Vec::with_capacity(children.len());
        for child in children {
            if is_chrome(&child, in_section) {
                child.parent.set(None);
            } else if is_element(&child, "img") {
                // Alt text carries what an image says; its URL, often a
                // data: URI, is noise to a reader of text.
                child.parent.set(None);
                let alt = attribute(&child, "alt").unwrap_or_default();
                let alt = alt.split_whitespace().collect::<Vec<_>>().join(" ");
                if !alt.is_empty() {
                    kept.push(text_node(&alt, &node));
                }
            } else {
                kept.push(child);
            }
        }
        *node.children.borrow_mut() = kept;
        // Resolve links against the final response URL, never an untrusted
        // <base> element. Keep only ordinary web destinations.
        if is_element(&node, "a")
            && let NodeData::Element { attrs, .. } = &node.data
            && let Some(href) = attrs
                .borrow_mut()
                .iter_mut()
                .find(|attr| attr.name.local.as_ref() == "href")
        {
            href.value = base
                .join(&href.value)
                .ok()
                .filter(|url| matches!(url.scheme(), "http" | "https"))
                .map(|url| url.to_string())
                .unwrap_or_default()
                .into();
        }
        pending.extend(
            node.children
                .borrow()
                .iter()
                .rev()
                .map(|child| (child.clone(), in_section)),
        );
    }
}

/// Whether a resolved link leads nowhere a reader could fetch: no safe
/// destination, or back into the page itself.
fn leads_nowhere(href: &str, page: &Url) -> bool {
    let Ok(mut url) = Url::parse(href) else {
        return true;
    };
    url.set_fragment(None);
    url == *page
}

/// Drops links with no text, such as heading permalinks, and keeps only the
/// text of links that lead nowhere. Runs after [`clean`] resolved every href.
fn simplify_links(document: &Handle, page: &Url) {
    let mut pending = vec![document.clone()];
    while let Some(node) = pending.pop() {
        let children = std::mem::take(&mut *node.children.borrow_mut());
        let mut kept = Vec::with_capacity(children.len());
        for child in children {
            if !is_element(&child, "a") {
                kept.push(child);
                continue;
            }
            // Permalinks often hold only a zero-width space.
            if text(&child)
                .chars()
                .all(|c| matches!(c, '\u{200b}'..='\u{200d}' | '\u{2060}' | '\u{feff}'))
            {
                child.parent.set(None);
            } else if leads_nowhere(&attribute(&child, "href").unwrap_or_default(), page) {
                child.parent.set(None);
                for grandchild in std::mem::take(&mut *child.children.borrow_mut()) {
                    grandchild.parent.set(Some(Rc::downgrade(&node)));
                    kept.push(grandchild);
                }
            } else {
                kept.push(child);
            }
        }
        *node.children.borrow_mut() = kept;
        pending.extend(node.children.borrow().iter().rev().cloned());
    }
}

fn serialize(node: &Handle) -> Result<String, String> {
    let mut bytes = Vec::new();
    let options = SerializeOpts {
        traversal_scope: if matches!(node.data, NodeData::Document) {
            TraversalScope::ChildrenOnly(None)
        } else {
            TraversalScope::IncludeNode
        },
        ..Default::default()
    };
    html5ever::serialize(&mut bytes, &SerializableHandle::from(node.clone()), options)
        .map_err(|_| "HTML serialization failed".to_owned())?;
    String::from_utf8(bytes).map_err(|_| "HTML serialization failed".to_owned())
}

pub(crate) fn convert(source: &str, base: &Url) -> Result<(String, String), String> {
    let document = parse(source);
    let title = descendants(&document)
        .find(|node| is_element(node, "title"))
        .map(|node| text(&node))
        .unwrap_or_default();
    clean(&document, base);
    simplify_links(&document, base);
    let body = descendants(&document)
        .find(|node| is_element(node, "body"))
        .unwrap_or_else(|| document.clone());
    let selected = descendants(&document)
        .find(|node| is_element(node, "main"))
        .or_else(|| descendants(&document).find(|node| is_element(node, "article")))
        .unwrap_or_else(|| body.clone());
    let extracted_chars: usize = descendants(&selected)
        .map(|node| match &node.data {
            NodeData::Text { contents } => contents.borrow().chars().count(),
            _ => 0,
        })
        .sum();
    // A thin main/article selection often means the content lives elsewhere.
    // Convert the whole cleaned body in that case, keeping Markdown links.
    let selected = if extracted_chars < 200 {
        body
    } else {
        selected
    };
    let converter = htmd::HtmlToMarkdown::builder()
        .skip_tags(vec![
            "head", "script", "style", "nav", "footer", "aside", "noscript", "svg",
        ])
        .build();
    let markdown = converter
        .convert(&serialize(&selected)?)
        .map_err(|_| "HTML to Markdown conversion failed".to_owned())?;
    Ok((title, markdown))
}
