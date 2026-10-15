use html5ever::serialize::{SerializeOpts, TraversalScope};
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{Handle, NodeData, RcDom, SerializableHandle};
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

fn clean(document: &Handle, base: &Url) {
    let mut pending = vec![document.clone()];
    while let Some(node) = pending.pop() {
        node.children.borrow_mut().retain(|child| {
            let remove = matches!(&child.data, NodeData::Element { name, .. }
                if matches!(name.local.as_ref(),
                    "script" | "style" | "nav" | "footer" | "aside" | "noscript" | "svg"));
            if remove {
                child.parent.set(None);
            }
            !remove
        });
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
