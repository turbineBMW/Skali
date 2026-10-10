# Skali — a native GTK4/libadwaita Basecamp client

Plan written 2026-10-10 from a survey of Rustle, Bubo and Vmux, the Basecamp 5
API docs (`basecamp/bc-api`), the official CLI (`basecamp/basecamp-cli`, Go),
the official SDKs (`basecamp/basecamp-sdk`, incl. the `basecamp-sdk` Rust crate)
and the Basecamp web app on a real account.

## 1. What we are building

A desktop Basecamp client for Linux in the spirit of Rustle (mail) and Bubo
(Google Messages): fast, keyboard-friendly, Omarchy-themed, works offline from a
local cache, and gets real-time updates. Not a web wrapper (every existing Linux
"client" is Electron around the web app; nothing native exists).

Observed in the real account: one project ("Example Project", 27 people) with a
Chat, a Card Table, Docs & Files, two Message Boards and three Chats. Multiple
instances of the same tool type per project is normal and must be supported.
Home shows a project grid, "Most recent activity", Pings and "Hey!" (New for
you), plus My Tasks / My Events / My Bookmarks / My Activity / My Notes.

## 2. Key facts from the research (decisions hang on these)

- **API**: `https://3.basecampapi.com/{account_id}/...`. Basecamp 5 flat routes
  are canonical (`/todos/123.json`, `/recordings/123/comments.json`); legacy
  `/buckets/{id}/...` still works. Mandatory `User-Agent: Skali (url or email)`.
  Geared pagination via `Link: rel="next"`. ETag / `If-None-Match` → 304.
  Rate limit ~50 req / 10 s per IP, 429 + `Retry-After`.
- **Auth**: two issuers.
  - Launchpad (`launchpad.37signals.com`): auth-code flow, requires
    `client_secret` at exchange, 2-week access tokens + refresh token. Loopback
    redirect `http://127.0.0.1:<port>/callback` is accepted (the official CLI
    uses `http://127.0.0.1:8976/callback`, and embeds its id/secret, calling
    them "public client credentials for the native CLI app"). This is the only
    self-service registration path today: https://launchpad.37signals.com/integrations
  - `app.basecamp.com` (OAuth 2.1, PKCE, device flow, DCR): live but called a
    "dark pilot" by the CLI README. Treat as a future upgrade, not the MVP.
  - Personal access tokens exist at `https://app.basecamp.com/my/access_tokens`
    (no refresh). Worth offering as a "paste a token" fallback.
  - `GET https://launchpad.37signals.com/authorization.json` lists accounts
    (`accounts[].href` is the base URL). First call after login.
- **Real-time**: `POST /events/stream_ticket.json` → Action Cable WebSocket
  URL, subscribe `EventsChannel` with `types` (e.g. `chat.line.created`,
  `comment.created`, `todo.*`, `card.*`, `message.*`). Pings ride
  `chat.line.created` too. Durable position comes from `GET /events.json`
  polling (`since=` / `position=`), ~30 s safety delay. Live events are thin
  pointers; refetch the recording. Webhooks exclude chat and need public HTTPS,
  so they are irrelevant to us.
- **Hey! / notifications**: `GET /my/readings.json` (unreads ≤100, sections
  `inbox|chats|pings|bubbles|mentions`), `PUT /my/unreads.json` marks read.
- **Pings**: `GET /chats.json` returns all campfires incl. pings; pings are the
  ones whose `bucket.type != "Project"` (the CLI's TUI heuristic). No endpoint
  to create a new ping; `GET /circles/people.json` lists pingable people.
- **Chat lines**: `GET /chats/2/lines.json`, paginated newest-first, no
  `since` param. Use the event feed to wake up, then refetch page 1.
- **Rich text**: sanitized HTML with `<bc-attachment sgid=...>` for files and
  @mentions. BC5 adds `content_attachments[]` siblings so files can be listed
  without parsing HTML. Uploads: `POST /attachments.json?name=` raw body →
  `attachable_sgid`.
- **Rust SDK**: `basecamp-sdk` 0.24.0 (tokio, reqwest/rustls, MSRV 1.88),
  generated from Smithy, 187 paths. Has an `oauth` module (PKCE, device,
  refresh). Lacks: `authorization.json`, ETag caching, Action Cable connector
  (Go only). Fast-moving 0.x.
- **Official CLI** (`basecamp-cli`, Go, MIT, AUR package): best reference for
  the auth state machine, loopback callback, keyring-with-file-fallback, and
  refresh-token rotation locking. `basecamp mcp` (stdio) exposes 16 domain
  tools; hosted MCP at `https://mcp.basecamp.com/mcp`.

## 3. Decisions

| Topic | Decision | Why |
|---|---|---|
| Layout | Cargo workspace: `crates/skali-core` (no GTK) + `crates/skali` (GTK) | Rustle pattern; core is unit-testable |
| Edition | 2024, `rust-version = "1.88"` | Bubo/Vmux already on 2024; SDK MSRV 1.88 |
| App id | `io.github.turbinebmw.Skali`, resource path `/io/github/turbinebmw/Skali` | Rustle's (newest) convention |
| UI | Blueprint `.blp` composite templates, `gio::ListStore` + `ListView`, `Adw.NavigationSplitView` | Rustle pattern, fits list/detail |
| Async | Global tokio runtime (`rt.rs` from Bubo) + `reqwest`; results back via `async_channel` → `glib::spawn_future_local` | Natural for HTTP, pagination, websockets |
| HTTP/API | Own thin client over `reqwest` with hand-written serde models for the subset we show. Vendor model structs from `basecamp-sdk` where convenient. Re-evaluate adopting the SDK wholesale after the auth spike. | We need ETag cache, `authorization.json`, Action Cable, and control over churn |
| Auth (MVP) | Launchpad auth-code + loopback listener (`tiny_http` or a few lines of tokio TCP) + `open::that`; embedded client id/secret like the CLI; PAT paste as fallback | Only self-service path today |
| Secrets | `oo7` Secret Service, Rustle's shared-collection module; JSON file fallback | Same as Rustle; CLI does the same |
| Settings | GSettings schema compiled into OUT_DIR for `cargo run` (Rustle `settings.rs`) | Proven |
| Cache | `rusqlite` (bundled), main-thread connection, append-only migrations, FTS5 for local search | Rustle `db.rs` |
| Rich text | WebKitGTK 6 view for messages/documents/comments (Rustle reader); Pango markup conversion for chat lines and excerpts | Rendering fidelity vs. list performance |
| Logging | `tracing` → stderr (`SKALI_LOG`) + always-on file log in `~/.cache/skali/` (Bubo `init_logging`) | Field debugging |
| Packaging | Cargo only, `install.sh`/`uninstall.sh`/`justfile` copied from Rustle; `.desktop`, metainfo, gschema, D-Bus + systemd user unit for `--hidden` background mode | House style |
| Theming | `accent.rs` + `omarchy.rs` ported from Rustle/Bubo, `follow-omarchy-theme` default on | House style |

## 4. Architecture

```
crates/skali-core
  api/        client.rs (reqwest, User-Agent, auth header, 429/5xx backoff,
              Link pagination, ETag store), endpoints/*.rs, models/*.rs,
              richtext.rs (HTML → Pango, bc-attachment expansion)
  auth/       launchpad.rs (authorize URL, loopback callback, exchange, refresh),
              tokens.rs (TokenManager: refresh 90 s before expiry, rotation lock),
              accounts.rs (authorization.json → Account list)
  feed/       cable.rs (tokio-tungstenite Action Cable client, ticket mint,
              reconnect w/ jitter), poll.rs (/events.json position loop),
              events.rs (typed Event enum → channel)
  db.rs       rusqlite schema + migrations (accounts, projects, dock, recordings
              by type, chat_lines, people, readings, etag cache)
  secrets.rs  oo7 keyring (+ file fallback)
  error.rs    thiserror ApiError { Http, RateLimited{retry_after}, Unauthorized,
              Network, Decode } + is_transient(); UI-facing Failure enum
  sync.rs     orchestrates initial load / refresh / event-driven refetch

crates/skali
  main.rs, config.rs, settings.rs, rt.rs, workers.rs (channel helpers), i18n.rs
  application.rs      adw::Application, single instance, --hidden, actions
  window/             mod.rs (State in RefCell), sidebar.rs, home.rs,
                      project.rs, tools/{chat,messages,todos,cards,docs,schedule}.rs,
                      hey.rs, assignments.rs, search.rs
  widgets/            chat_line_row, message_row, todo_row, card, person_chip,
                      richtext_view (WebKit), composer (sourceview5 + libspelling)
  objects.rs          GObject wrappers over core models for ListStore
  ui/*.blp            templates
  notify.rs, accent.rs, omarchy.rs, autostart.rs
```

Data flow: UI action → `workers::call(async fn)` on tokio → result over
`async_channel` → main thread updates SQLite + ListStores. Event feed task pushes
`Event`s into an unbounded channel drained on the main thread; each event marks
the affected recording stale and schedules a refetch (debounced per bucket).

Navigation model: `Adw.NavigationSplitView` with sidebar = accounts ▸ projects
(+ pinned Home, Hey!, Pings, My Stuff); content = `Adw.NavigationView` stack:
project overview (dock cards) → tool → item → comments. Breadcrumb titles from
the recording `parent` chain.

## 5. Phases

### Phase 0 — Scaffold (½ day)
- Workspace, both crates, `config.rs`, `build.rs` (blueprint + gresource +
  schema), gschema with window geometry + `follow-omarchy-theme`, metainfo,
  desktop file, icons placeholder, `install.sh`/`uninstall.sh`/`justfile`,
  `CLAUDE.md` (Rustle rules adapted), CI from Vmux.
- Empty window with "No account" StatusPage. `just check` clean.

### Phase 1 — Auth spike (1–2 days)
- Register Launchpad integration (redirect `http://127.0.0.1:8976/callback`
  or a port we pick; also register `localhost`). Needs the real account.
- Launchpad flow end to end: open browser, loopback catch with `state`
  check, exchange, store in keyring, `authorization.json` → accounts.
- TokenManager with refresh; PAT paste dialog as fallback.
- Decide SDK vs own client after trying `basecamp-sdk`'s oauth/client on
  the real account (keep whichever needed less glue).

### Phase 2 — Read-only browsing (1 week)
- Projects list (`/projects.json` + `/my/recent_projects.json`), project page
  rendering the `dock[]` with `enabled` tools, grouped by type.
- Message board → messages → comments (WebKit richtext view).
- Campfire: lines list (newest-first pages, reverse for display), people
  avatars, @mention expansion, image attachments inline.
- To-dos: todoset → lists → groups → todos (+ completed toggle view).
- Card table: columns → cards, steps.
- Docs & Files: vault tree, documents, uploads (download via `download_url`).
- Schedule: upcoming entries.
- SQLite cache behind everything so re-opening is instant; ETag conditional
  GETs on refresh.

### Phase 3 — Me-centric views (3–4 days)
- Hey!: `/my/readings.json`, sectioned list, mark-read on open.
- Pings: campfires with non-Project bucket; conversation view.
- My Assignments (`/my/assignments.json`), My Bookmarks, My Notes, Timeline
  (`/reports/progress.json`).
- Search: `/search.json` with type filters, plus FTS5 over local cache.
- Desktop notifications for new unreads (Bubo `notify.rs`), with click → open.

### Phase 4 — Writes (1 week)
- Post chat line (plain + simple HTML), post ping reply.
- Comment on any recording; boosts.
- Complete / reopen / assign / reschedule to-dos; add to-do.
- Move card between columns; complete card.
- New message (composer: markdown-ish editor → sanitized HTML; attachments via
  `/attachments.json` → `<bc-attachment>`).
- Subscribe/unsubscribe, bookmark, mark read.

### Phase 5 — Real-time + background (3–4 days)
- Action Cable client: mint ticket, subscribe with `types` for the open
  views (+ `chat.line.created` always when running), 10 s confirm deadline,
  reconnect with jittered backoff and fresh ticket.
- `/events.json` poll every ~60 s for the durable position; reconcile.
- `--hidden` background mode via systemd user unit; tray-less, notification-
  driven like Bubo.

### Phase 6 — Polish and ship
- Omarchy theme following, accent colours, keyboard shortcuts
  (Shift+J jump like the web app), i18n `pot`, screenshots, README,
  AUR `skali-git` PKGBUILD.

## 6. Status (2026-10-10)

Done:
- Phase 0, 1: scaffold; Launchpad sign-in with refresh, keyring, accounts.
- Phase 2: projects, Campfire (scroll-back), message boards, to-dos, card
  tables, Docs & Files, Schedule (upcoming by day from
  `/reports/schedules/upcoming.json`, recurring occurrences, what's due),
  avatars, inline images, project cache.
- Phase 3: Hey! (mark read on open, mark all read), Pings, My Assignments,
  My Bookmarks (unstar from the list), Timeline (`/reports/progress.json`, by day),
  Search (`/search.json`: as you type, by kind, best match or newest;
  Ctrl+F), desktop notifications. Hey! and My Assignments rows open chats, messages,
  cards, docs, uploads and to-do lists in Skali (`links::AppLink`), other
  links in the browser; so do Basecamp links in rich text and chat lines. A
  link to a comment scrolls to it.
- Phase 4: chat lines, comments, new messages, file attachments on messages
  and comments; complete/reopen, add, assign and reschedule to-dos; move a
  card to any column or straight to Done (Basecamp's way of completing
  one), tick card steps; boosts on recordings and their comments (add with
  a few emoji or words, take yours back), bookmark and subscribe toggles.
- Phase 5: live feed (Action Cable + poll) for open chats and recordings;
  `--hidden` and autostart.
- ETags: every JSON GET is conditional on the ETag it last saw (per account,
  in memory, LRU-bounded); a 304 reuses the kept body and `Link`/count. The
  quiet feed poll gets 304s.
- Phase 6: Omarchy theme following, accent, `po/` template, Preferences.

Open, in rough order:
- Local search (FTS5 over the cache, for offline), My Notes.
- Writes: boosts on chat lines, to-do notes; dragging cards on the board.
- PAT paste fallback for sign-in.
- Keyboard shortcuts, screenshots, README, AUR `skali-git` PKGBUILD.

## 7. Development aids

- Install `basecamp-cli` from AUR and run `basecamp auth login` once. Then
  `claude mcp add basecamp -- basecamp mcp --read-only` gives the coding agent
  live read access to the real account while building views.
- `basecamp --json projects list` etc. is a quick way to see real payload
  shapes without writing code.
- OpenAPI 3.1 spec for models: https://github.com/basecamp/basecamp-sdk/blob/main/openapi.json

## 8. Decided (were open questions)

1. Launchpad integration: registered (as "Campfe"; rename it to "Skali" at
   https://launchpad.37signals.com/integrations, same id/secret) with redirect
   `http://127.0.0.1:8976/callback`; id/secret embedded in `config.rs`.
2. SDK vs own client: own thin client.
3. Commits: Conventional Commits; AI co-author trailers allowed.
4. MVP: chat, message boards, to-dos, Hey! first; card tables and docs next
   (both since done).
5. Name (2026-10-10): Skali (Old Norse *skáli*, a hall or hut), not Campfe,
   which read too close to Basecamp's own Campfire. App id, data dir, keyring
   schema and env vars moved with it; the first run moves the old data dir,
   cache and keyring item over.

## 9. Sources

- API: https://github.com/basecamp/bc-api (sections/authentication.md,
  event_feed.md, campfires.md, my_notifications.md, rich_text.md, search.md…)
- CLI: https://github.com/basecamp/basecamp-cli (internal/auth/auth.go,
  callback.go), toolkit https://github.com/basecamp/cli
- SDKs: https://github.com/basecamp/basecamp-sdk, https://crates.io/crates/basecamp-sdk
- MCP: https://github.com/basecamp/mcp, https://basecamp.com/ai
- Reference code: ~/Projects/Rustle (install.sh, build.rs, settings.rs,
  secrets.rs, db.rs, window/, ui/main-window.blp), ~/Projects/Bubo (rt.rs,
  ui/mod.rs, gm/client.rs, ui/login.rs, main.rs init_logging, notify.rs),
  ~/Projects/graphmail-bridge (oauth.rs TokenManager)
