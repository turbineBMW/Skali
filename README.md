# Campfe

A native Basecamp client for Linux: GTK 4, libadwaita, Rust. Your projects, Campfire chats,
message boards, to-dos, card tables and files in a fast desktop window, with live updates
from Basecamp's event feed.

Status: early. Signs in through Launchpad and lists your projects. See PLAN.md for where
it's going.

## Build

```
just run       # from a checkout (needs gtk4 ≥ 4.18, libadwaita ≥ 1.8, blueprint-compiler)
just install   # release build into ~/.local
```

Signing in needs a Launchpad integration (https://launchpad.37signals.com/integrations) with
the redirect URI `http://127.0.0.1:8976/callback`; build with its credentials in
`CAMPFE_OAUTH_CLIENT_ID` and `CAMPFE_OAUTH_CLIENT_SECRET`.

## License

GPL-3.0-or-later.
