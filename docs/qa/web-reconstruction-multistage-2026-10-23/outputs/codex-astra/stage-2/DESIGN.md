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
