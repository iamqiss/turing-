# Turing Tarpit TUI (`tt`)

Ratatui terminal client — black + orange `#ff3c00`.

```bash
cargo run -p tt-tui
```

Config: `~/.config/turing-tarpit/tui.toml`

## Auth
- Sign in / **F2** create account (passphrase register)
- Session token stored locally

## Navigation
| Key | Action |
|-----|--------|
| 1–6 / h l | Screens / tabs |
| Mouse tap | Tabs, repo rows |
| r | Refresh |
| L | Logout |
| q / Ctrl-C | Back / quit |

## Repos screen
| Key | Action |
|-----|--------|
| j k | Move |
| Enter | Detail |
| **w** | Create **workspace** (name + slug) |
| **n** | Create **repo** (name, description, private/public) |
| **d** | Delete selected repo (confirm) |
| [ ] | Cycle target workspace for create |

APIs: `POST/GET /api/workspaces`, `POST/GET/DELETE /api/workspaces/:id/repos`
