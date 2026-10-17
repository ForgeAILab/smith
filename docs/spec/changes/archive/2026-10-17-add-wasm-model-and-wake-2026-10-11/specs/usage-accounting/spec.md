## ADDED Requirements

### Requirement: Module model usage is accounted

Smith SHALL record the provider usage of every model call made by a
sandboxed module under an attribution naming that module, and SHALL include
it in the session's usage and cost at every surface that reports them.

#### Scenario: Cost after a module model call

- **GIVEN** a module made a model call that reported usage
- **WHEN** the user runs `/status`
- **THEN** the session's usage and cost include that call
- **AND** it is attributed to the module
