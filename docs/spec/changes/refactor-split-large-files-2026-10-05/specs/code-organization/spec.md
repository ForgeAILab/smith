## ADDED Requirements

### Requirement: Rust source files stay within a line budget

Every `.rs` file under `crates/` SHALL be 1,500 lines or less. A workspace
test MUST enforce the limit and name each file over it with its line count.
Splitting a file to meet the limit MUST keep its module path, public items,
and signatures, MUST NOT widen an item's visibility beyond what its sibling
modules need, and MUST NOT add a test binary.

#### Scenario: A file grows past the limit

- **GIVEN** a change that makes a `.rs` file under `crates/` 1,501 lines long
- **WHEN** the workspace tests run
- **THEN** the line-budget test fails
- **AND** its message names that file and its length

#### Scenario: A large file is split

- **GIVEN** a source file over the limit
- **WHEN** it is split into child modules under the same module root
- **THEN** existing import paths and public signatures still compile
- **AND** the moved tests run with unchanged names and assertions
- **AND** recorded fixtures do not change

#### Scenario: An integration test file is split

- **GIVEN** an integration test file `tests/<name>.rs` over the limit
- **WHEN** it is split
- **THEN** it becomes `tests/<name>/main.rs` with child modules
- **AND** the crate builds the same number of test binaries as before
