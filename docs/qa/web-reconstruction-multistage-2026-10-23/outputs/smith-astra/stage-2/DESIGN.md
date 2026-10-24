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

## Revision 2 — authoritative single-frame design
The heading is now “Good things are taking shape.” and the marker is BOARD 9D3E6B, positioned immediately after the longer heading (approximately x1007 at desktop). Revenue is $31,420, occupancy 76%, active members 146, available desks 24. Occupancy uses a red #d60000 down-right arrow and -8.0%; other trends remain green. Utilization uses a 76% lime arc with “21 of 28 spaces active”; the ring center is approximately (1320,516).

Keep the original column widths and metric/chart row positions. Refine lower-row start to y691 (13px gap after the 330px chart row), and the account divider to y916 with avatar y941–989. The footer label is at approximately y988; this supersedes the initial y1004 estimate above. Promotion is now “Room to” / “grow.” with approximately 46px Georgia, compact 0.94 line-height, and “Find your next space” CTA; retain the supporting sentence and illustration. Promo top padding is 43px. Ring top margin reduces to 9px to align its center. On intermediate and mobile widths allow the longer heading and marker to wrap without horizontal overflow. All interaction handlers and booking table records remain unchanged; the renamed promotional CTA still selects Spaces.

