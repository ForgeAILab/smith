## ADDED Requirements

### Requirement: Server-namespaced remote tool rows

A remote tool row SHALL identify both its server and its tool so the user can
tell which third party is being invoked. The row MUST attribute the call to a
server even when two servers advertise the same tool name.

#### Scenario: Two servers advertise the same tool

- **GIVEN** calls to a tool named `search` on two different servers
- **WHEN** their rows render
- **THEN** each row identifies its own server
- **AND** the two rows are distinguishable

### Requirement: Remote tool arguments are hidden by default

Smith MUST NOT display the arguments of a remote tool call by default, because
argument fields are defined by the server and Smith cannot classify which carry
sensitive values. The row SHALL show that arguments were withheld rather than
omitting the fact.

#### Scenario: Remote call carries an opaque argument

- **GIVEN** a remote tool called with a server-defined argument object
- **WHEN** its row renders
- **THEN** the row shows the namespaced tool name
- **AND** states that arguments are hidden
- **AND** does not render any argument value

#### Scenario: Live and resumed rows agree

- **GIVEN** a remote tool call rendered during a live session
- **WHEN** the same session is resumed from persisted state
- **THEN** the resumed row shows the same namespaced name and hidden-argument
  marker
