## ADDED Requirements

### Requirement: Plugin installation is hash-bound trust

Smith MUST NOT load a plugin's content until the user has confirmed an
inventory of what it would load, shown with a digest covering the manifest,
every loaded component file, and its resolved MCP declarations. The decision
SHALL be bound to that digest. When installed content no longer matches the
recorded digest, the plugin MUST be withheld and reported as changed until
confirmed again. Plugin content covered by a plugin confirmation MUST NOT
prompt again per skill, command, or server.

#### Scenario: Install shows what will load

- **WHEN** the user runs `smith plugin add ./deploy-tools`
- **THEN** Smith lists each skill and command, each MCP server's resolved
  command line or URL, and each component it will not load, with the digest
- **AND** installs nothing until the user confirms

#### Scenario: Installed content is edited

- **GIVEN** an installed, trusted plugin
- **WHEN** a file in its install directory is modified
- **THEN** the plugin is reported as changed and none of its content loads
- **AND** the session starts without it

#### Scenario: Update changes an MCP command line

- **GIVEN** an installed plugin at one version
- **WHEN** `smith plugin update` fetches a version with a different MCP
  command line
- **THEN** Smith shows the difference and asks for confirmation
- **AND** the installed version keeps running until the user confirms

### Requirement: Plugin sources and store

Smith SHALL install plugins from a local directory, from
`github:owner/repo[#ref]`, and from a git URL, and SHALL resolve
`name@marketplace` through a registered marketplace's
`.claude-plugin/marketplace.json` for relative-path, GitHub, and git-URL
entries. Installation MUST copy the plugin into the user state root; Smith
MUST NOT load a plugin from its source checkout. Each plugin SHALL have a
data directory that survives updates. A symbolic link that resolves outside
the plugin root MUST be refused. An unsupported source type MUST be reported
by name.

#### Scenario: Install from a marketplace

- **GIVEN** a registered marketplace `acme` listing plugin `deploy-tools`
  with a GitHub source
- **WHEN** the user runs `smith plugin add deploy-tools@acme` and confirms
- **THEN** the plugin is fetched, copied into the plugin store, and recorded
  with its source, resolved commit, version, and digest

#### Scenario: Git is unavailable

- **GIVEN** `git` is not on the user's PATH
- **WHEN** the user installs from a git source
- **THEN** Smith fails with an error naming `git`
- **AND** installs nothing

#### Scenario: Unsupported source type

- **GIVEN** a marketplace entry with an npm source
- **WHEN** the user installs it
- **THEN** Smith reports that npm sources are not supported

### Requirement: Plugin enablement is configuration

Smith SHALL decide whether an installed plugin loads from
`plugins.<name>.enabled`, resolved through the ordinary layered resolution
with provenance, defaulting to the manifest's `defaultEnabled` and otherwise
to on. A key naming a plugin that is not installed MUST be reported and MUST
NOT fail resolution. In non-interactive runs, enabled trusted plugins SHALL
load, and Smith MUST NOT install, update, or trust a plugin without an
interactive confirmation or an explicit `--yes`.

#### Scenario: Plugin switched off in a profile

- **GIVEN** an installed plugin that is on by default
- **AND** the selected profile sets `plugins.deploy-tools.enabled = false`
- **WHEN** configuration resolves
- **THEN** the plugin is off and its provenance names the profile

#### Scenario: Headless run with a changed plugin

- **GIVEN** an enabled plugin whose content no longer matches its digest
- **WHEN** `smith -p` runs
- **THEN** the plugin's content is not loaded
- **AND** the run continues and reports the withheld plugin
