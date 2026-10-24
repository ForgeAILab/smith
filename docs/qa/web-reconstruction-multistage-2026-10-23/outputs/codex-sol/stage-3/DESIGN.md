# Wayfinder reference reconstruction

## 1. Visual contract
The three original 1536px-wide slices form one 1536×1024 desktop page; revision.png is now authoritative where it differs. Preserve all current wording, counts, relative geometry and the selected Overview/Week states. Current heading: “Good things are taking shape.”; marker: BOARD 9D3E6B. The dark forest sidebar, lime accents, rounded white panels and architectural doorway illustration define the reference. No new visual direction. Energy 2, rhythm 2, motion 0: static initial presentation.

## 2. Tokens
- Canvas #faf9f6; panel #fff; ink #09131c; forest #0b3325; muted #687079; borders #d7dce0.
- Selected lime #d9ff96; ring lime #cbff60; green change #00820b; negative change #dc160a; confirmed #daf6ce; pending #ffcfbf with #ad2c13 text.
- Secondary illustration greens: #e0f8af, #8da985, #41694d. Avatar fills: #d8f6cf, #ffccbe, #bfd9ff; account #d4fbe5.
- Panel and primary control radius 10px; tag radius 6px; status radius 15px; circular avatars.
- Main spacing: left 39px, right 22px, row gaps 20–21px; metric gap 18px; panel padding 24px; sidebar inset 20px.
- Fine line weight 1px; chart stroke 2.5px; icon stroke 1.8px. White panels use barely perceptible white-to-cool-white shading, with no heavy shadows. Forest surfaces have subtle tonal shading matching the reference.

## 3. Typography
Use local Arial/Helvetica sans-serif without remote fonts. Body 16px, small labels 14px; metric labels 16px and values 42px bold; heading 50px/60px bold with -1.3px tracking; subtitle 24px/29px. Section titles 24px/29px bold with -0.6px tracking. Logo 21px uppercase with 4px tracking. Promotional display uses Georgia 48px/42px, tightly tracked; “Room to” and “grow.” occupy two lines. Keep 34px/37px on small mobile screens. Promo supporting text 18px/26px. Chart ticks and table cells 14px. Utilization value 40px bold.

## 4. Layout
Desktop sidebar occupies x=0–267 and full viewport height. Logo at (29,31), building control at (20,106), nav starts y=178 with 64px pitch. Account divider at y=916 and account row y=940; desktop sidebar bottom padding is 36px. Main x=306–1514. Breadcrumb at y=31, date/notification/create controls y=19–63. Heading top y=66 and subtitle y=127. The longer heading retains its 50px size, with the board marker near x=1006; allow heading/marker wrapping before they exceed the available width. Metrics y=180–327 in four equal columns. Activity and utilization y=348–676 in 806px/388px columns with 14px gap. Lower row y=691: table 729px wide, promo 467px wide, gap 13px; panels end near y=964/967. Footer y=983.
Activity plot has horizontal ticks 0,20,40,60; smooth forest line through Mon≈11, Tue≈22, Wed≈34, Thu≈38, Fri≈42, Sat≈29, Sun≈13; pale green area, Friday selected lime dot and anchored dark tooltip. Utilization ring uses round ends with 76% lime (458.4px of the 603.2px circumference) and a muted green track.

## 5. Reusable primitives and states
- SVG icons with consistent stroke and no raster assets; purely decorative icons hidden from assistive technology.
- Sidebar buttons: icon and label, dark idle, lime selected, subtle hover, visible keyboard focus; aria-current identifies selection.
- Buttons: forest primary, outlined date/building, flat text links; hover brightness and visible focus. Building and date use native accessible popovers/inputs when activated. Account and notifications expose small informational popovers.
- Panels: shared border/radius/background; metrics share label/value/detail anatomy.
- Period segmented buttons: Week selected initially, aria-pressed, Month changes labels and data.
- Booking rows: semantic table, circular initials and semantic status text.
- Booking form: native modal dialog with labelled name, space, date/time, status; required validation, cancel/save, Escape dismissal, focus return. Submission appends a row and updates footer status.
- Promo: semantic text plus decorative inline SVG doorway, pendant lamp, plant and architectural lines. Find your next space selects Spaces. View all expands the booking list state.

## 6. Responsive behavior
At widths below 1250px reduce sidebar to 220px and main padding to 24px; metrics stay four columns until 1000px. Below 900px sidebar becomes a full-width header with wrapping navigation; content becomes one column with 16px inset; metrics form two columns; chart and utilization stack. At 375px no fixed minimum content widths: main heading wraps, controls wrap, plot scales via viewBox, booking table reflows into labelled member cards, and promo stays readable. Nothing causes horizontal viewport overflow.

## 7. Accessibility and verification
Semantic aside/nav/main/header/section/table/footer. Text remains DOM text, charts have accessible descriptions. Controls have explicit names, focus outlines, and minimum 40px touch targets where feasible. Native dialog supplies focus containment and Escape support. No animation or network access. Verify desktop geometry, 375px overflow, period state, navigation selection and create/cancel/submit keyboard interactions. Browser QA depends on locally available tooling; do not install dependencies or run background servers.

Validation: HTML parsed successfully; identifiers are unique; observations JSON parses; JavaScript syntax check passes; no external or reference-image assets are used. Local browser automation packages and browser executables were unavailable, so rendered browser QA could not be performed. Desktop coordinates and mobile CSS constraints were checked from source.

Revision 2 evidence: observations-2.json records the updated heading, marker, all four metrics, red downward occupancy indicator, 76% utilization with 21 active spaces, larger promo heading and new CTA. Booking data and activity chart are unchanged. observations-1.json is retained as historical evidence.

Illustration geometry refinement: outer arch starts near x=1303 and y=718; doorway near x=1363; pendant near x=1391; plant pot near x=1303. Shift arch, doorway, lamp and plant 29px left within the existing illustration while retaining right-side wall lines and floor extents.

## 8. Shared shell and Bookings page
The app has two distinct documents: index.html retains the revised Overview, and bookings.html implements bookings.png. Both use the same local shared stylesheet, sidebar, icon symbols, panel/metric/status primitives, and native dialog styling. Overview and Bookings navigation items are real relative anchors with aria-current on the current page. Other navigation items retain selection behavior. Links inherit the exact navigation geometry, font and colors without underlines.

Bookings contract: heading “Every booking, in view.”, marker “BOOKINGS C6E2A8”, subtitle “Plan the week. Keep everyone in sync.” The top date reads “Oct 23, 2026”. New booking is lime with forest text on this page. Metrics are Total bookings 48 (+12%), Rooms booked 12 (+8%), Pending approval 6 (+50%). All three use green upward indicators. Grid columns approximately 387/387/400px with 17px gaps; y=180–328.

Bookings main content starts y=347. Left panel x=306, width=853, height=619; right panel x=1174, width=340, same height; gap=15. Left padding=22, toolbar y=371, height=44. Segmented filter width=313; search width=229; space selector width=144. Table starts y=449 with a 34px header and 65px rows, ending y=873. Columns approximately 27%, 23.5%, 24%, 19%, 6.5%. Avatar size=45, member gap=18; status chips 32px high. New avatar fills: purple #e7caff, yellow #fff0b3, mint #b9efd7. Pagination footer is y=900–943, with left summary and right 40px controls. Use 14px table text, 16px footer text and consistent muted labels.

Right panel padding=19px, title 24px, month heading 16px. Week row has seven equal columns; selected Fri 23 has an 84px-high lime rounded tile and dark dot. Month row y=435, week row y=459–543, separator y=565. Next up title 20px at y=596; next card x=1194 y=629 width=300 height=131, border/radius matching panels. Card avatar 49px; name 18px bold; space 16px muted; time 14px muted with a calendar icon. Full-width lime View calendar action sits at y=888, height=56, bottom inset=21.

Interactions: All bookings, Confirmed and Pending are mutually exclusive filter buttons with aria-pressed; search matches member or space case-insensitively and combines with status and space filters. Empty results show readable text. Added bookings join the dataset, update visible metrics/summary and can be filtered. Only the six screenshot records are initially available; retain the screenshot aggregate of 48 without inventing 42 unseen records. Pagination can expose an explicit unavailable-records empty state. Row action buttons and Next up open an accessible booking detail dialog. Week arrows change displayed week and day buttons select a day. View calendar opens an accessible month calendar dialog. New booking uses the native modal form, required validation and Escape dismissal on both pages.

Responsive: below 1250px the toolbar wraps and table text can reduce; below 1100px the list and week panels stack. At 540px and below, toolbar controls fill available width, the table becomes semantic stacked rows with member, space, date/time, status and actions visible; pagination wraps. Week columns remain seven equally sized cells with no fixed minimum width. At 375px the three metrics stack vertically, and all panels, inputs, dialogs and links stay within the viewport.

Final checks for page 2: both documents parse with unique IDs and valid relative assets/links. Dashboard revision content assertions pass. Bookings logic checks pass for status filters, case-insensitive search combined with space/status, empty results, dialog open/cancel, new-row submission and totals, filtering newly added rows, pagination, week navigation, and month calendar navigation. JavaScript syntax passes for both pages. Calendar test expectation was corrected to include all 31 October dates plus three leading cells and seven weekday labels. Rendered browser QA remains unavailable in this environment; 375px behavior was reviewed from responsive CSS.
