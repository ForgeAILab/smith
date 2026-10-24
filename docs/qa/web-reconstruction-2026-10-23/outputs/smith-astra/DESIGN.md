# Screenshot-derived design contract

## Canvas and layout
- Reference canvas: 1536 × 1024 CSS pixels. Warm ivory page, 267px full-height forest-green sidebar. Main content has 39px left and 22px right inset.
- Sidebar: wordmark at (29,31), property selector at (20,106), six navigation rows beginning y178 with 64px cadence; selected row is 226 × 51px. Account block sits at the bottom below a thin divider at y916.
- Main toolbar: breadcrumb at (306,32), right-aligned date, notification bell, and booking button. Hero heading begins near (306,66); small outlined board marker sits beside it. Subtitle below.
- Four equal metric cards span x306–1514, y180–328, with 18px gaps and 10px corner radii.
- Chart row begins y347 and is 330px tall: activity panel 807px wide, 14px gap, utilization panel 389px wide.
- Bottom row begins y691: bookings table approximately 729px wide, 13px gap, promo card approximately 467px wide. Height 275px. Footer at y988.

## Tokens
- Forest: #0b3023; deep text: #0a141c; warm canvas: #faf9f6; panel: #ffffff.
- Lime active navigation: #daff96; ring lime: #caff5e; pale promo green: #e2f9b4.
- Secondary text: #686f77; borders: #d8dce0; trend green: #087a18.
- Confirmation pill: #dcf7ce; pending pill: #ffd0c2 with rust text. Avatar fills: green, peach, and pale blue.
- Panel radii 10px; buttons 9px; status pills 13px. Most borders are 1px; charts and icons use delicate 1.5–2px strokes. No strong shadows except the chart tooltip.

## Typography
- No external fonts. Use Arial/Helvetica sans-serif, matching the reference's neutral sans shapes. Body 16px; sidebar 16px; breadcrumb, axes, legend and table 14px.
- Hero: 50px, 700–800 weight, slightly negative letter spacing. Subtitle: 24px medium gray.
- Metric numbers: 44px bold; metric captions 16px. Panel headings: 24px bold with tight spacing.
- Wordmark: 21px, wide 4px tracking. Promo headline uses Georgia serif at 42px with tight 42px line height and two lines.

## Reusable primitives and graphics
- Semantic buttons with inline SVG outline icons; compact card headings; border-only white cards; rounded initial avatars and status badges.
- Activity chart: seven dark green points connected by a smooth dark green curve, a light lime area fading down to white, horizontal dotted gridlines at 0/20/40/60, weekday labels, and an initially emphasized Friday point with a dark tooltip. Plot spans roughly x365–1088 and y439–624.
- Utilization: dark card, heading at top left, 206px diameter circular gauge centered around (1320,516), lime rounded 84% arc over a translucent green track. Center contains a large white 84% and smaller active-spaces caption. Three-color legend at bottom.
- Promo: pale green panel, left-aligned text, right-side fine-line archway illustration with forest doorway, lamp, perspective lines, and leafy potted plant. Draw the decorative artwork independently as inline SVG, not a traced full-page image.

## States and behavior
- Preserve the pictured Overview, Week, October 23 2026, three booking rows, and Friday tooltip on initial load.
- Navigation updates the active selection and breadcrumb. Property selector offers the visible property without inventing locations. Date control permits date changes. Notifications and account controls open small dismissible popovers.
- Week/Month segmented buttons update selected state and chart labels/data. New booking opens a native accessible modal with member, space, date/time, and status inputs. Valid submission appends a semantic table row and updates the timestamp. View all reveals the complete list when new rows exist. Explore spaces activates Spaces navigation.
- Dialog has a labeled title, native focus containment, autofocus, Cancel and close controls. Escape closes the dialog and returns focus to its opener.

## Responsive behavior
- Desktop proportions follow the reference; intermediate widths reduce main padding and allow the heading and toolbar to wrap.
- At narrow widths the sidebar becomes an in-flow compact navigation region, cards form two columns and then one column as needed, and chart/bottom regions stack. Chart SVG scales within its panel. Table uses wrapped content and narrower spacing rather than causing horizontal page overflow.
- At 375px all controls remain usable, table text remains visible, and decorative promo art stays clipped to its own panel.

## Accessibility
- Landmarks for navigation and main content; actual headings and table column headers; buttons rather than clickable noninteractive elements.
- Descriptive accessible names on icon-only controls, aria-current on selected navigation, aria-pressed on period buttons, chart title/description and textual value summary.
- Strong visible focus outlines; decorative icons hidden from assistive technology; dialog form labels and required fields; polite live status for successful bookings. Respect reduced motion; no animation required.
