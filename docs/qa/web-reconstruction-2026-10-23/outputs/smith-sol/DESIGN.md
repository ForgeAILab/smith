# Screenshot design contract

## Visual tokens
- Reference viewport: 1536 × 1024 CSS pixels.
- Canvas: warm ivory `#faf9f6`; card white `#fff`; ink `#0b1620`; forest `#0b3325`; secondary text `#69717c`; borders `#d9dde0`.
- Accent: pale lime `#d8ff91`; utilization ring `#caff60`; promotion `#e2f9b8`; positive text `#087c19`; confirmed pill `#d9f7ce`; pending pill `#ffd0c2` with rust text.
- Standard border: 1px; corner radius: 9–11px; restrained or absent shadows.
- Spacing units: 4, 8, 12, 16, 20, 24, 32, 40px.

## Layout
- Full-height 267px forest sidebar. Main starts at x=306 (39px inset), right inset 22px.
- Sidebar logo near (29, 31), property selector at (20, 106), 226×45. Navigation starts y=178 with 52px items and 12px gaps. Account footer is bottom-aligned above a 35px margin.
- Top breadcrumb y=32. Right toolbar y=19, 44px tall. Title y=65, subtitle y=127. Board marker sits alongside title.
- Four equal metric cards at y=180, height 148px; 18px gaps. Each has 24px padding.
- Middle row y=348, height 329px: booking activity takes approximately 67%, utilization 32%, with 14px gap.
- Bottom row y=691: bookings table approximately 60%, promotional panel 39%, gap 13px. Heights 273–276px.
- Footer update note at y=988.

## Typography
- System sans-serif (Arial-compatible), no downloaded fonts. Body 16px, table and small controls 14px.
- Title: 50px, bold, tight tracking, forest green. Subtitle: 24px medium gray.
- Metric values: 42px bold. Card headings: 24px bold with slightly tight tracking.
- Logo: 21px uppercase with 4px letter spacing. Promo headline: 42px Georgia/serif, 1.0 line-height.

## Reusable primitives and artwork
- Outline navigation icons at 24px with round strokes; compass brand mark drawn in lime SVG.
- Card with subtle gray border, 10px radius. Metric label/value/change stack.
- Chart segmented Week/Month buttons, pale-lime active segment. SVG chart with dashed horizontal guides at 0/20/40/60, smooth green line, area gradient, dark dots, lime Friday highlight and anchored tooltip.
- Utilization card uses forest ground and 206px circular ring, centered percentage and caption, three-dot legend.
- Semantic table with thin row rules, colored circular initials, and rounded status pills.
- Promotional artwork is locally drawn vector architecture: tall arched green doorway, fine green construction lines, hanging pendant, potted plant, steps. Not a screenshot asset.

## States and behavior
- Initial navigation Overview, chart Week, Friday tooltip visible, three original bookings visible.
- Navigation updates the active highlight and breadcrumb; leaves dashboard content available.
- Week/Month updates chart data and selected state. Chart points expose daily values accessibly.
- New booking opens a native modal dialog with labeled member, space, date/time and status inputs. Valid submission appends a semantic table row. Cancel/Escape dismiss; focus returns to opener.
- View all reveals the complete booking list; Explore spaces selects Spaces. Property/account and notification controls provide local, dismissible feedback. Date control opens a local date picker.

## Responsive behavior and accessibility
- At narrower desktop widths, reduce sidebar and main padding; preserve readable cards.
- At mobile widths, sidebar becomes a compact top region, navigation wraps; main content is one column, metrics two columns, bottom panels stacked. Table uses compact wrapping rather than horizontal scrolling.
- Use semantic aside/nav/main/header/section/table, labeled buttons, SVG titles, aria-current navigation and aria-pressed period state.
- Visible focus outlines; native dialog supplies focus containment, keyboard dismissal, and modal semantics. Respect reduced-motion preference. No external requests, dependencies, assets, or reference-image embedding.
