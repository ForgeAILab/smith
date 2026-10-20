# Skills, file commands, plugins, MCP servers, and modules

## Skills

A skill is `<dir>/<name>/SKILL.md` with `description` frontmatter. The
directory name is the skill name.

- User skills: `~/.smith/skills/<name>/SKILL.md` or
  `~/.agents/skills/<name>/SKILL.md`.
- Project skills: `.smith/skills/<name>/SKILL.md` or
  `.agents/skills/<name>/SKILL.md`. These need project trust. The user
  types `/skills trust <name>`.
- `/skills` lists what is indexed, including shadowed and untrusted skills.

You may create a user or project skill with your edit tool. A skill only
adds instructions; it cannot grant tools or permissions.

## File commands

`~/.smith/commands/<name>.md` (or `.smith/commands/<name>.md` in a project)
becomes the command `/<name>`. The body is a prompt template. `/commands`
lists them, `/commands trust <name>` trusts a project command, and
`/commands reload` re-reads the files.

## Plugins

Plugins are installed from the command line and never from inside a
session:

- `smith plugin add <path|github:owner/repo|git-url>` prints a review and
  asks to install and trust. The user must confirm, or pass `--yes`.
- `smith plugin list` shows each plugin's state. You may run it.
- `smith plugin update [name]` and `smith plugin remove <name>` manage
  installed plugins.
- In a session, `/plugins <name> on` or `/plugins <name> off` previews the
  change, asks to confirm, and applies it at the next safe boundary.

## MCP servers

Declare a server in config:

```toml
[mcp.servers.github]
command = "github-mcp"
args = ["stdio"]
env = { GITHUB_TOKEN = "env:GITHUB_TOKEN" }

[mcp.servers.docs]
url = "https://mcp.example.com/mcp"
credential = "env:DOCS_TOKEN"
```

Use credential references, not literal tokens. A new stdio server stays
stopped until the user types `/mcp trust <name>`. `/mcp` shows each
server's state and its last stderr lines when it fails.

## Modules

Modules are compiled-in features such as `vision`, `image-generation`, and
`web`. `/modules` and `smith config modules` show each one as mounted or
inactive, with the reason. When a fix is available, they also show a hint.
`/modules <id> on` or `/modules <id> off` edits the user config, asks to
confirm, and rebuilds the session.

## Capabilities

`/capabilities` lists the tools and skills offered in this session.
`/capabilities deny tool:<name>` withholds one for the session. To make it
permanent, use `[profiles.<name>.capabilities] deny = [...]`.
