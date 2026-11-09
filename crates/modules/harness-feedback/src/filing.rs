use crate::Report;
/// Public issue text and its bounded prefilled URL.
#[derive(Debug, Clone)]
pub struct FilingPreview {
    /// One-line title, bounded to 120 characters.
    pub title: String,
    /// Exact public issue body to preview.
    pub body: String,
    /// Prefilled GitHub URL; building it performs no network request.
    pub url: String,
}
/// Build exactly the public text the review surface will show before hand-off.
pub fn github_issue(report: &Report) -> FilingPreview {
    let title = crate::store::bounded(&report.input.title, 120);
    let e = &report.evidence[0];
    let sections = [
        format!("## What happened\n{}", report.input.what_happened),
        format!("## User said\n{}", report.input.user_said),
        format!("## Repro\n{}", report.input.repro),
        format!(
            "## Environment\nSmith: {} ({})\nOS: {}\nProvider/model: {}/{}\nArea: {}",
            e.identity.version,
            e.identity.commit,
            e.os,
            e.identity.provider,
            e.identity.model,
            report.input.area
        ),
    ];
    let url_for = |body: &str| {
        let mut url =
            url::Url::parse("https://github.com/ForgeAILab/smith/issues/new").expect("static URL");
        url.query_pairs_mut()
            .append_pair("title", &title)
            .append_pair("body", body)
            .append_pair("labels", "harness-feedback");
        url.to_string()
    };
    let marker = format!(
        "(truncated; run smith feedback show {} for the full report)",
        report.id
    );
    let mut body = sections.join("\n\n");
    if url_for(&body).len() > 8000 {
        body.clear();
        for section in sections {
            let candidate = if body.is_empty() {
                section
            } else {
                format!("{body}\n\n{section}")
            };
            if url_for(&format!("{candidate}\n\n{marker}")).len() > 8000 {
                break;
            }
            body = candidate;
        }
        body.push_str(&format!("\n\n{marker}"));
    }
    FilingPreview {
        url: url_for(&body),
        title,
        body,
    }
}
