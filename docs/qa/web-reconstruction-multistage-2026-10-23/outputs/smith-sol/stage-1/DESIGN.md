# Wayfinder reconstruction

## Evidence and geometry
The three reference slices form a 1536 × 1024 page. A full-height dark green sidebar ends at x=267. Main content begins at x=306 (39px inset), with a 21px right gutter. Header breadcrumb sits at y=32; title baseline area y=65–111, subtitle y=130. Metrics span y=180–327. Chart row spans y=355–685. Bottom row spans y=691–967, followed by an update note at y=988. Sidebar navigation continues across slices; profile is anchored near the bottom.

## Tokens
- Canvas: warm off-white #faf9f5; panels nearly white #fff.
- Forest: #0c3326; near-black text #09131b; muted text #666e77.
- Highlight: pale lime #dbff96; chart ring #caff64; positive #087617.
- Borders: #d8dde0, 1px; panel radius 9–11px; no pronounced shadows.
- Spacing: 8, 12, 16, 20, 24, 28, 40px.

## Typography
Use local Arial/Helvetica sans-serif (no network fonts). Logo is uppercase widely tracked, 21px. Main heading 50px/1.15 bold, slight negative tracking. Subtitle 24px. Card numbers 42px bold. Section headings 24px bold with tight tracking. Body 16px; labels, chart ticks, breadcrumb and table headers 14px. Promotional heading uses Georgia serif, 42px/1.02.

## Layout and primitives
- Sidebar 267px wide, padding 20px. Compass logo, outlined location switcher, six icon-and-label navigation buttons. Selected item has lime fill. Bottom divider and circular avatar/profile control.
- Main padding 20px 21px 24px 39px. Top toolbar places breadcrumb opposite date, notification and booking button. Four equal metric cards with 18px gaps.
- Middle grid roughly 2.08:1, 13px gap. Activity card contains title and segmented control plus an SVG area/line chart with seven points. Utilization uses a dark card and circular progress chart.
- Bottom grid roughly 1.56:1, 13px gap. Semantic booking table with avatar/name rows and colored status pills. Pale lime promotional panel has an original inline SVG architectural doorway, pendant lamp and plant illustration at right.
- Small icons are inline SVG, stroke-based. Reuse icon definitions, metric cards, status pills and buttons.

## States and behavior
Initial state exactly reflects screenshots: Overview selected; Week selected; Friday chart tooltip visible; three bookings. Navigation selection changes the active visual state without replacing dashboard content. Period control changes chart labels/data and selection. New booking opens a native accessible dialog with name, space, datetime and status fields; valid submission appends a semantic table row and updates feedback. Escape and Cancel close it. Other small controls expose local menus/status feedback; no network calls.

## Responsive and accessibility
At narrow widths sidebar becomes a compact top section with wrapping navigation, grids collapse, toolbar wraps and table becomes horizontally compact with wrapping cell text rather than viewport overflow. At 375px cards form a single column. Preserve desktop layout at 1536px. Use buttons for actions, labeled inputs, aria-current/aria-pressed selection states, accessible SVG labels, visible keyboard focus, native modal focus trapping and live submission feedback. Decorative illustration and icons are hidden from assistive technology.

## Verification notes
Recovery confirmed that index.html is complete. Static checks passed for observations JSON parsing, HTML parsing, screenshot text, unique IDs, SVG symbol references, JavaScript syntax, and absence of external/reference-image asset URLs. A dependency-free Node DOM harness also passed sidebar selection, month/week chart switching, dialog open/cancel, booking-row creation, whitespace-only name rejection, and Escape dismissal of the date control. The booking form uses a native dialog for modal focus and Escape behavior. No browser executable or local Playwright/Puppeteer package was available; rendered desktop/mobile geometry, viewport overflow, and real browser interaction testing remain unverified. No packages were installed and no server or background process was started.
