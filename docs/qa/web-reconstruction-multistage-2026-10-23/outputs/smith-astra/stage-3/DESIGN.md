# Wayfinder reconstruction

## Evidence and layout
The three supplied slices form one 1536 × 1024 dashboard. Sidebar is 267px wide, full height. Main content starts at x306, ends at x1514, with 39px left and 22px right padding. Header occupies y19–151. Four metric cards occupy y180–328. Second row y348–676: activity panel 806px wide, 14px gap, utilization 388px. Third row y692–968: bookings 729px, 14px gap, promotional card 466px. Footer note at y1004. Sidebar account divider sits at y900.

## Tokens
- Canvas warm ivory #faf9f5; panels near white #ffffff; borders #d8dcde.
- Dark evergreen #0b3326; ink #0a141b; muted #697078.
- Selection pale lime #d8ff91; chart accent #c8ff60; promotional fill #e2f8b7.
- Corners: panels 9px, primary controls 9px, badges 13px.
- Spacing: 14px row gaps, 24px card interior, 20px sidebar inset.
- Sans typography approximated using Arial/Helvetica system fonts (no external fonts). Display title 50px bold with -1.5px tracking; subtitle 24px; metric values 42px bold; panel headings 24px bold; body 16px; metadata 14px. Promotional headline uses Georgia at 42px with tight line height.

## Reusable primitives
Outlined panels, icon/text navigation items, metric cards, pill status badges, two-option segmented control, avatar circles, outlined secondary and evergreen primary buttons. Icons are small inline SVG strokes. Booking chart uses an SVG area fill with a smooth evergreen curve; highlighted Friday value is 42. Utilization ring has a muted track and a lime 84% arc. Promotion has independent SVG architectural/plant illustration, not a page-sized tracing.

## States and interactions
Initial Overview and Week selected. Sidebar selection moves lime highlight and updates breadcrumb without changing initial dashboard content. Chart period buttons switch selected state and chart data. New booking opens a native modal dialog with labelled inputs; submit adds a semantic table row and updates the footer. Escape closes the dialog. View all expands the booking table and reports its current row count; Explore spaces selects Spaces. Location/date/account/notification controls expose small contextual popovers.

## Responsive behavior and accessibility
At narrow widths sidebar becomes a compact top navigation area, content uses reduced padding, metrics use two columns, and chart/utilization/table/promotion stack. At 375px table uses smaller type and wrapping to avoid page overflow. Native buttons provide keyboard activation; selected states use aria-current or aria-pressed. Native dialog supplies modal focus containment and Escape handling; labels, required fields, close control and focus restoration support accessible booking entry. SVG charts have text alternatives; decorative icons are hidden from assistive technology. 

## Page 3 — Bookings and shared primitives
The Bookings reference specifies a distinct second page, not a replacement dashboard. Preserve all Revision 2 dashboard content and chart/form interactions. Extract existing CSS without visual changes to `shared.css`, retaining the same evergreen sidebar, ivory canvas, system fonts, metric cards, badges, avatars, icons, native dialog, and header geometry. Overview and Bookings become native local anchors on both pages, with `aria-current="page"` on the current page. Non-page sidebar entries remain selection controls. Page-specific rules are scoped to `.bookings-page` or live in `bookings.css`.

At 1536 × 1024 the Bookings shell retains the 267px sidebar and main x306–1514. Heading: “Every booking, in view.”, marker BOOKINGS C6E2A8, subtitle “Plan the week. Keep everyone in sync.” Three 148px-high metric cards begin y180, with 17px gaps, showing 48, 12, 6. The New booking button is lime with dark text on this page only. Date label is abbreviated “Oct 23, 2026”. Bookings navigation selection is approximately y237–289 (slightly higher than the current dashboard Bookings item).

Main row begins y347. Left panel is x306–1158 (852px), right calendar panel x1174–1514 (340px), with 16px gutter; both are 619px high. Left inset is 22px. Toolbar begins (328,371), height43, with three segmented filters (313px), search (229px) and space select (144px). Table heading begins near y456, first row boundary y483; six rows are approximately65px each. Table has member, space, date/time, status and an unlabeled visible row-action column with accessible button names. Avatars are 45px, status pills approximately104×32px. Footer starts near y900; count on left, page controls on right. Dataset beyond the six visible records is not specified; never invent extra members to fill the displayed total48. Pagination may show an honest empty state for pages without supplied records while new rows are added to the local dataset.

Calendar inset20px. Heading at y374, month at y428, weekday strip at y459–543. Friday23 has a lime 44×84px selected tile and dark dot. Divider y565. Next up title at y596, bordered member card y629–760. Lime View calendar button occupies x1194–1493 and y888–944. Purple/yellow/mint avatar colors extend the existing three avatar colors. Use small SVG chevrons, search and ellipsis icons.

All bookings/Confirmed/Pending are real combinable filters with case-insensitive member/space search and a space select. Counts and empty states update on interaction, leaving the initial six-row appearance intact. Native dialog adds a confirmed/pending row using safe DOM text, resets filters to reveal it, and closes with Escape. Calendar arrows shift the visible week; date tiles select a day; View calendar opens an accessible month dialog. Row menu allows status changes/removal. Filters use `aria-pressed`, search and selectors have explicit accessible labels, updates are announced in a polite live region, and controls have visible keyboard focus. At375px cards/toolbar/columns stack, navigation stays compact, and the semantic table wraps within the viewport with reduced text/avatars instead of horizontal page scrolling.

## Revision 2 — authoritative single-frame design
The heading is now “Good things are taking shape.” and the marker is BOARD 9D3E6B, positioned immediately after the longer heading (approximately x1007 at desktop). Revenue is $31,420, occupancy 76%, active members 146, available desks 24. Occupancy uses a red #d60000 down-right arrow and -8.0%; other trends remain green. Utilization uses a 76% lime arc with “21 of 28 spaces active”; the ring center is approximately (1320,516).

Keep the original column widths and metric/chart row positions. Refine lower-row start to y691 (13px gap after the 330px chart row), and the account divider to y916 with avatar y941–989. The footer label is at approximately y988; this supersedes the initial y1004 estimate above. Promotion is now “Room to” / “grow.” with approximately 46px Georgia, compact 0.94 line-height, and “Find your next space” CTA; retain the supporting sentence and illustration. Promo top padding is 43px. Ring top margin reduces to 9px to align its center. On intermediate and mobile widths allow the longer heading and marker to wrap without horizontal overflow. All interaction handlers and booking table records remain unchanged; the renamed promotional CTA still selects Spaces.

