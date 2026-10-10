# CLAUDE.md

Skali is a native GTK 4 / libadwaita Basecamp client written in Rust. App id
`io.github.turbinebmw.Skali`, GSettings schema, data dir `$XDG_DATA_HOME/skali`, keyring
schema `io.github.turbinebmw.Skali.Token`, D-Bus name. The plan and the research behind it
are in PLAN.md; the API reference is https://github.com/basecamp/bc-api.

## Build & run

No Flatpak, no meson. `cargo run -p skali` works from a checkout: `build.rs` compiles the
Blueprints into a GResource and the GSettings schema into `OUT_DIR`, and `settings::load`
falls back to that compiled schema when the app isn't installed.

```
just run           # cargo run -p skali
just check         # clippy -D warnings + rustfmt --check + cargo test  (keep it clean)
just install       # sh install.sh -> ~/.local (PREFIX=/usr for system-wide)
```

Host deps: gtk4 ≥ 4.18, libadwaita ≥ 1.8, webkitgtk-6.0 ≥ 2.48, blueprint-compiler, glib2
(glib-compile-schemas).
sqlite is bundled by rusqlite; TLS is rustls. Edition 2024, MSRV 1.88.

`SKALI_LOG=debug` (any `tracing` EnvFilter) turns stderr logging up; `~/.cache/skali/skali.log`
always has debug. To exercise the app without touching real data: `XDG_DATA_HOME=<dir> cargo run`.

Dev hooks: `skali api [GET|POST|PUT|DELETE] <path> [json]` makes one raw call with the stored
sign-in and prints the body (writes obey the guard below). `SKALI_DEBUG_OPEN=<project>[:<tool>[:first]]`
or `=hey|pings|assignments|search[:<words>]`, or `=<app_url>` (routed as a Hey! row is) opens
that page at launch. The sandbox project is 2000001 with Chat 10392702064, Message Board
10392702077, To-dos 10392702093 and Schedule 10392772614. For a screenshot on Hyprland:
`hyprctl clients -j` for the window geometry, `grim -g`; `wtype` types into the focused widget.
Only text, Tab and Return get through `wtype` here (modifiers, BackSpace and Escape don't), and
the portal file chooser takes none of it: `SKALI_DEBUG_ATTACH=<file>` starts every attachment
picker with that file instead. `hyprctl dispatch` takes Lua:
`hyprctl dispatch 'hl.dsp.focus({ window = "class:io.github.turbinebmw.Skali" })'`.

**Sign-in uses Skali's own Launchpad integration**, whose public client id/secret are in
`config.rs` (as 37signals' CLI ships its own). `SKALI_OAUTH_CLIENT_ID` / `SKALI_OAUTH_CLIENT_SECRET`
at build or run time swap in another; its redirect URI must be exactly
`http://127.0.0.1:8976/callback`.

**Never write to a real project while developing.** Run with `SKALI_WRITE_ACCOUNTS=1000002` (the
sandbox account "Sandbox", sandbox@example.com, a separate Launchpad identity): the API client then refuses POST/PUT/DELETE to every other account
(`ApiError::ReadOnly`). The Example Co account (1000001) is real people's work; test posts go to
the sandbox account only. The same goes for the `basecamp` CLI and the Basecamp MCP in this
session: reads only against Example Co.

## Layout

```
Cargo.toml                 workspace (edition 2024, thin LTO in release)
crates/skali-core/src/     no widgets; everything here is unit-tested (`cargo test`)
  api.rs       reqwest Client per account: User-Agent, bearer, Link pagination, 429/5xx
               retries, ETag cache (If-None-Match, 304 -> kept body), the
               SKALI_WRITE_ACCOUNTS guard
  auth.rs      Launchpad OAuth: authorize URL, loopback callback listener, exchange, refresh,
               authorization.json (identity + accounts). TokenSet's Debug hides the tokens.
  secrets.rs   oo7 keyring: one JSON TokenSet per Launchpad identity
  db.rs        rusqlite; schema + append-only MIGRATIONS tracked by PRAGMA user_version
  models.rs    serde structs for what we read (Authorization, Account, Project, DockTool, Person)
  error.rs     ApiError (+ is_transient)
  feed.rs      the event feed: stream ticket + Action Cable WebSocket (run_stream), the poll lane,
               Event. Thin pointers: refetch the recording before showing anything.
  richtext.rs  HTML -> Pango markup / plain text / excerpt; expand_attachments for the WebKit view;
               text_to_html for composing
  omarchy.rs   Omarchy theme palette -> CSS;  darkmode.rs  HTML dark-mode rewrite (for the
               rich-text view)
crates/skali/src/          the GTK layer
  main.rs      logging (tracing: stderr + file), gettext, resources, run
  application.rs  SkaliApplication: db + settings, --hidden, app actions
  rt.rs        the tokio runtime and `rt::run(job, on_done)` -- the threading model (below)
  window/      SkaliWindow: mod.rs (State, sidebar, projects, Hey! count + notifications),
               sign_in.rs (OAuth flow, refresh), navigate.rs (pushing pages), live.rs (one feed
               per account; registry of open chat/message pages that live events update)
  pages/       one module per page kind: project, chat (ChatView: paging, composer, live
               lines), messages (board list; MessagePage: WebKit document of body + comments,
               images inlined as data: URIs), todos, hey, pings, assignments
  dialogs/     preferences.rs (accounts with sign-out, theme and notification switches)
  widgets/     richtext_view.rs: the WebKit view (JS off, CSP, links to the browser, dark-mode
               adapted);  avatars.rs  avatar fetch + disk cache
  ui/*.blp     Blueprint templates; the Rust attribute names must match the ids
  settings.rs  GSettings keys;  config.rs  ids, OAuth app;  i18n.rs  gettext + failure_message
  accent.rs / omarchy.rs  accent colour and Omarchy theme following (ported from Rustle)
data/                      gschema, desktop file, metainfo, D-Bus + systemd user units, icons
po/                        skali.pot (`just pot`) and <lang>.po; install.sh compiles them
```

## Rules

- **Threading:** all network and keyring I/O goes through `rt::run(job, on_done)`: `job` is a
  `Send` future on the tokio runtime with owned data (a cloned `Client`, ids); `on_done` runs on
  the main loop and is the only place that touches the database or widgets. Never log a token.
- **Window state** is one `RefCell<State>` behind `state()`/`state_mut()`. Never hold a `Ref`
  across a call that may need `state_mut()`. Bind to a local before an `if let`/`match`: a
  temporary in the scrutinee lives for the whole block.
- **Basecamp manners:** every request carries the User-Agent; listings follow `Link rel=next`
  (never build page URLs); 429 waits `Retry-After`; GETs go through the client's ETag cache. Flat
  BC5 routes (`/todos/1.json`) are canonical; `/buckets/...` is legacy.
- **Several tools of one kind per project is normal** (three Chats, two Message Boards):
  key everything on the tool id, never on the tool kind.
- **Errors surfaced to the user are also logged** at the `on_done` boundary, naming the
  resource; the sentence comes from `i18n::failure_message`.
- **No account is a real state:** the window has a welcome StatusPage; everything per-account
  reads `State::accounts` through a guard.
- **CSS colours** come from libadwaita variables (`var(--accent-bg-color)`) so Omarchy themes
  reach them.
- **DB migrations** are append-only.
- Commits: Conventional Commits, terse. AI co-author trailers are fine here.
