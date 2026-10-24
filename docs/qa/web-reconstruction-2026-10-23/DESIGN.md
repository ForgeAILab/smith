# Screenshot benchmark viewer

## 1. Purpose and identity
An evidence viewer for one screenshot-reconstruction experiment. Actual reference and browser captures carry the visual information. Controls and typography stay quiet so differences between implementations are easy to see. Benchmark outputs remain unedited.

## 2. Color
`--paper: #f5f4ef`, `--panel: #ffffff`, `--ink: #102f25`, `--muted: #5e6c65`, `--line: #d4ddd6`, `--accent: #d9fa8a`. Forest ink on white or lime gives readable controls. White panels separate evidence from the warm page surface.

## 3. Typography
System sans-serif, 16px body with 1.55 line height. Display 36px, panel titles 22px, compact labels 14px. Numeric metrics use tabular figures. No external font requests.

## 4. Layout
Maximum width 1480px with 24px desktop gutters, 16px mobile gutters. Main comparison uses the reference's 3:2 aspect ratio. A 2-column capture grid becomes one column below 700px. Controls wrap without horizontal overflow; the data table gets its own scroll container.

## 5. Components and states
Buttons have default, hover, selected, active, disabled and visible focus states; minimum height 44px. Native model selection and an opacity slider support pointer and keyboard input. The comparison stage has reference, reconstruction and overlay modes. Capture cards link to the unchanged HTML implementations. A disclosure explains the methods and limitations. Missing outputs show an explicit unavailable state rather than an invented screenshot.

## 6. Responsive behavior and motion
Keep screenshots proportional at every width. On mobile, controls stack and labels remain visible. No animation is needed for this inspection task; the opacity slider responds immediately. Desktop evidence uses 1536×1024, with separate 768px and 375px browser captures.

## 7. Accessibility and evidence
Use semantic headings, table headers, labels, descriptive image alternatives and visible keyboard focus. The slider's current value is announced by its native range control. Contrast follows the palette above. The report describes one trial per model/harness combination and does not claim general model rankings or isolated latency measurements.
