# Wayfinder screenshot reconstruction

## 1. Visual contract
Reconstruct the 1536 × 1024 dashboard. revision.png supersedes the original three slices wherever they differ; observations-2.json records current copy and values. Retain observations-1.json as the original evidence. Forest green navigation and utilization panel, warm off-white workspace, lime selected controls and architectural illustration. No external assets. Energy 2, rhythm 2, motion 0; fidelity takes precedence over stylistic changes.

## 2. Tokens
Colors: forest #0c3225; deeper forest #092e21; page #faf9f5; white #fff; ink #09131b; muted #667079; border #d6dadd; selected lime #d9fc94; chart lime #c9fc61; positive #008315; negative #dc0000; success background #dcf7d0; pending #ffd0c2; pending ink #ae290e. Avatar mint #d9f7d0, peach #ffcebf, blue #c5ddff.
Spacing: 4, 8, 12, 16, 20, 24, 28, 32, 40px. Card radii 9–11px. Border 1px. No large shadows.

## 3. Layout
Desktop sidebar 267px wide, full viewport minimum height 1024px. Main starts x306, right margin 22px. Top controls y19, heading y65, subtitle y128. Metrics y180, height147, 18px gaps. Chart row y347, height330, 14px gap, columns approximately 2.08:1. Bottom row y691, heights274–277, 14px gap, columns1.56:1. Footer y982. Sidebar logo y30; selector y106; navigation y178 at63–64px rhythm; account anchored at bottom beneath divider y916, avatar y941. The longer revised heading places the board marker near x1007 without shifting the metrics. Utilization ring center is approximately (1320,516), radius96 with14px stroke; the lime arc shows76%.

## 4. Typography
Local Arial/Helvetica sans-serif approximates screenshot. Body16px; small14px; heading50px bold with -1.4px tracking; subtitle24px; panel headings24px bold with -0.7px tracking; metric42px bold; ring40px bold. Logo20px with4px tracking. Revised promotion48px Georgia serif with43.2px line-height, split as “Room to” / “grow.”; preserve the supporting text near y843 and CTA near y914. Mobile promotion remains38px.

## 5. Reusable primitives
Outlined white cards, forest buttons, lime selection buttons, inline 24px line icons, rounded status badges, 40px circular initials. Metric cards contain label/value/change. Table uses semantic headers and rows. Charts use separate inline SVG with accessible labels; illustration is decorative inline SVG. Selection states use aria-current or aria-pressed. Hover uses subtle tonal shift, keyboard focus uses visible green outline. Dialog uses labeled native fields, required validation, cancel and submit controls, Escape dismissal and focus restoration. Successful submission appends a table row and announces completion.

## 6. Responsive behavior
Below1100px reduce main padding and type sizes, retain proportional columns. Below760px sidebar becomes a top navigation region, navigation wraps into a three-column grid; account and selector remain available. Main uses16px gutters, two metric columns, single chart/table/promotion columns. At375px table cells wrap into four compact columns without horizontal overflow. Top controls wrap. Chart SVG scales to card width.

## 7. Accessibility and behavior
Semantic aside/nav/main/header/section/table. All buttons keyboard operable. Decorative icons hidden from screen readers. Chart has textual accessible summary. Native modal traps focus and closes on Escape. Sidebar changes selected item and breadcrumb; Week/Month updates chart and selection; new booking opens form and adds a booking. Other visible controls expose local menus or relevant selection. No network, dependencies, reference-image loading, or background services.

## Revision 2
Current heading: Good things are taking shape. Marker: BOARD 9D3E6B. Occupancy uses a red downward arrow and -8.0% text; direction is conveyed by both shape and text. Utilization is76%, 21 of28 spaces active. Promotion CTA is Find your next space. Existing semantic structure, mobile reflow, selection controls, and booking modal remain.

## Bookings page and shared primitives
Keep index.html as the revised overview; bookings.html is a separate document. Both pages share sidebar, brand, location/account controls, header geometry, icon symbols, metric cards, status chips, and native booking dialog. Overview and Bookings are real relative links; aria-current identifies the current page. Remaining sidebar buttons retain selection behavior.
Bookings heading: Every booking, in view. Marker: BOOKINGS C6E2A8. Subtitle: Plan the week. Keep everyone in sync. The date uses abbreviated Oct. New booking uses lime with forest text on this page. Three metrics occupy x306–1513, y180–327, with18px gaps.
Bookings body starts y347: left panel x306 width852 height618; right calendar x1174 width340 height618, gap16. Left panel padding22px; toolbar y371 height43, segmented tabs width313, search229px, space select144px. Table header y462, rule y483, six rows65px tall through y873. Member avatars45px; table body15px; columns approximately27%,23.5%,24%,19%,6.5%. Pagination aligns y900, with count at left and outlined arrow buttons plus lime page1 at right. Additional avatars use lavender #e7c8ff, pale yellow #fff0b0, mint #b6f0d6.
Calendar panel padding20px; heading24px; month row at y434; seven equal day columns with selected Friday lime, rounded12px, height84px. Divider y565. Next up heading20px; booking preview card y630 height130 with48px avatar and labeled calendar icon. View calendar button is anchored21px above panel bottom, height56px and lime.
Tabs use aria-pressed and filter actual rows. Search matches member and space case-insensitively, combined with status and space filters. New bookings append to the visible data and can be filtered. Empty results have an explicit table message. Only six bookings are provided by the image; later numbered pages show an honest empty state rather than invented records. Calendar arrows move the displayed week, date buttons select a date, row menus expose booking details.
Below1100px toolbar wraps and calendar moves beneath the table when needed. Below760px the shared compact navigation is retained; metrics stack, toolbar controls wrap, table switches to labeled row cards, pagination wraps, calendar fills available width. No horizontal overflow at375px. Shared nav links use the same default, hover, selected and keyboard-focus appearance as the original buttons.
