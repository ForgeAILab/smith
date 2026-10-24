# Wayfinder reconstruction

## Evidence and geometry
The three reference slices form a 1536 × 1024 page. A full-height dark green sidebar ends at x=267. Main content begins at x=306 (39px inset), with a 21px right gutter. Header breadcrumb sits at y=32; title baseline area y=65–111, subtitle y=130. Metrics span y=180–327. Chart row spans approximately y=347–677. Bottom row spans y=691–967, followed by an update note at y=988. Sidebar navigation continues across slices; profile is anchored near the bottom.

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

## Revision 2 — authoritative revision.png
- Retain the original palette, sidebar, toolbar, card widths, table rows and chart data. Main title is now “Good things are taking shape.” with badge “BOARD 9D3E6B” at approximately x=1007; use a 12px title/badge gap and allow wrapping at smaller widths.
- Revenue is $31,420 (+18.4%); occupancy 76% (-8.0%); active members 146 (+18); available desks 24. Introduce a red #d00000 negative-trend state with a southeast arrow; retain the positive green treatment.
- Utilization ring and accessible label must both indicate 76%, with “21 of 28 spaces active”. For radius 97, use an approximately 463px arc out of a 610px circumference.
- Correct earlier geometry inference: the activity card begins at y=347, not y=355. Set a 19px gap after metric cards, 330px activity/utilization heights, and 14px before the bottom row. Increase activity top inset to 18px so its grid remains at y=439 while moving the card outline upward. Utilization begins 1px below the activity card. Its legend and ring move about 8px higher than the initial implementation.
- Bottom row remains at y=691. Move the utilization ring 3px higher within its panel while preserving the legend baseline using 7px top / 10px bottom margins. Promo now reads “Room to / grow.” with approximately 48px Georgia type and 42px line-height, and “Find your next space”. Retain the existing 277px panel, supporting copy and architectural illustration.
- At 375px the longer title wraps naturally inside the content width; preserve single-column cards, wrapping navigation, compact table and existing accessible form behavior.

## Page 3 — Bookings and shared shell
- Keep index.html as the revised Overview page, including all Revision 2 content and chart/form behaviors. Add bookings.html as a separate document. Overview and Bookings are real links on both pages; only the current link gets the lime active state and aria-current="page". The other navigation entries retain their local selection behavior.
- Extract the existing CSS without changing its dashboard rules into shared.css. Both pages reuse the same 267px sidebar, logo/icons, profile, toolbar, typography, borders, avatars, status pills, dialog and responsive shell. Scope Bookings-specific overrides to .booking-page so the dashboard remains unchanged.
- Bookings heading is “Every booking, in view.” with “BOOKINGS C6E2A8”; subtitle “Plan the week. Keep everyone in sync.”; toolbar date is abbreviated “Oct 23, 2026”. The New booking button is lime on this page only.
- Three metric cards span x=306–1514, y=180–327, with approximately 17px gaps. Content below starts y=347: a left list panel 853px wide and a right week panel 340px wide, separated by 15px. Both have a 619px minimum desktop height.
- List panel inset 21px. Filters/search toolbar y=371, 44px high: 313px segmented tabs; search 229px; space select 144px. Table header line is y=483 and six 65px rows end at y=873. Member cells use 44px avatars and a roughly 18px name gap. Use five columns, with an accessible visually hidden label for the actions column. Footer is at y=900 with result count and pagination.
- Right panel has 19px inset: “This week”, a month row with arrow controls, seven equal day buttons (Friday 23 highlighted), a divider, “Next up” and Jamie Chen's bordered card. Lime View calendar action is bottom-aligned at y=888 with 56px height.
- Tabs filter loaded rows by status; search matches member or space without case sensitivity; the space select composes with both filters. Preserve the screenshot's initial six rows and “Showing 1–6 of 48 bookings”; do not invent hidden booking identities. Pagination may explain unavailable records rather than fabricate data. New rows are added to the visible local data, totals update, and filters are reset so the added row is visible. Row actions open booking details. Date/week/calendar controls work locally without network requests.
- At narrow widths stack the week panel below the table, wrap toolbar controls and footer, allow cells to wrap, and reduce avatar/text dimensions. At 375px use one metric column and a compact semantic table without horizontal viewport scrolling. Use native dialog focus/Escape behavior, visible focus indicators, labeled search/select controls, pressed filter states, and live result feedback.

## Verification notes
Revision 2: JSON parsing, revised content/value checks, 76% arc/accessibility text, negative-arrow styling, HTML structure, unique IDs/SVG references and JavaScript syntax passed. A dependency-free Node DOM harness passed sidebar selection, chart period switching, dialog opening/cancellation, booking submission, whitespace-name rejection and revised available-desk feedback. This is not browser-level verification; native Escape behavior, screenshot matching and 375px overflow remain unverified because no local browser tooling is available.

Revision 1 verification (historical): Static checks passed for observations JSON parsing, HTML parsing, screenshot text, unique IDs, SVG symbol references, JavaScript syntax, and absence of external/reference-image asset URLs. A dependency-free Node DOM harness also passed sidebar selection, month/week chart switching, dialog open/cancel, booking-row creation, whitespace-only name rejection, and Escape dismissal of the date control. The booking form uses a native dialog for modal focus and Escape behavior. No browser executable or local Playwright/Puppeteer package was available; rendered desktop/mobile geometry, viewport overflow, and real browser interaction testing remain unverified. No packages were installed and no server or background process was started.
