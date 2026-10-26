## MODIFIED Requirements

### Requirement: Web fetch tool

Smith SHALL provide a `web_fetch` tool that GETs one `http` or `https` URL and
returns its content as text. HTML SHALL be reduced to its main content and
converted to Markdown with links preserved. The main content SHALL be the
page's `main` element, else its `article`, else its body, falling back to the
body when the chosen element holds under 200 characters. Conversion SHALL leave
out page chrome and invisible elements: scripts, styles, `nav`, `footer`,
`aside`, `noscript`, `svg`, a `header` that is outside `article`, `aside`,
`main`, `nav`, and `section` and holds no `h1`, interactive controls, frames,
embedded media, elements whose `role` marks navigation, a banner, a footer,
complementary content, search, a menu, or a dialog, and elements marked
`hidden`, `aria-hidden="true"`, or inline `display: none`. An image SHALL be
replaced by its alt text. A link with no text SHALL be removed, and a link to
the same page or to no safe `http`/`https` destination SHALL keep only its
text. `text/*`, JSON, and XML SHALL be returned as is. `image/*` SHALL be
returned as an image part. Any other content type SHALL be a tool error naming
the type. The tool SHALL bound each fetch at 5 MiB, 30 seconds, and 5
redirects. It SHALL send no cookies or credentials, and SHALL use a fixed Smith
User-Agent. The result SHALL begin with the final URL, status, content type,
and page title, followed by a line marking the body as external, untrusted
content from that URL.

#### Scenario: Read a documentation page

- **WHEN** the agent calls `web_fetch` with `https://docs.rs/serde/latest/serde/`
- **THEN** the result begins with the final URL, `200`, and `text/html`
- **AND** the body is Markdown of the page's main content with its links

#### Scenario: A release post with heading permalinks and images

- **GIVEN** a blog post whose headings each hold an empty link to their own
  anchor, whose site banner is a top-level `header`, and whose figures are
  images with alt text
- **WHEN** the agent fetches it
- **THEN** each heading is its text alone, with no empty link
- **AND** the site banner is absent
- **AND** each image appears as its alt text, without its URL
- **AND** links to other pages keep their destinations

#### Scenario: Oversized download

- **GIVEN** a URL serving 40 MiB
- **WHEN** the agent fetches it
- **THEN** Smith stops reading at 5 MiB and returns the converted prefix
  with a note that it was truncated

#### Scenario: Unsupported type

- **WHEN** the URL serves `application/zip`
- **THEN** the tool returns an error naming `application/zip`
