# Skali

A native Basecamp client for Linux, built with GTK 4, libadwaita and Rust. Your projects,
Campfire chats, message boards, to-dos, card tables, docs and schedule in a fast desktop
window, updated live from Basecamp's event feed. *Skáli* is Old Norse for a hall.

Skali is an independent project. It is not made or endorsed by 37signals.

## What it does

- **Projects**: every tool in a project's dock. Projects often have several Chats or Message
  Boards, and each one gets its own page.
- **Campfire and Pings**: scroll back through history, post lines, see new ones arrive live,
  and boost a line from its hover button.
- **Messages, cards, documents, uploads, schedule entries**: rich text with inline images,
  comments with attachments, boosts, bookmark and subscribe toggles. A Basecamp link opens
  in Skali when Skali has a page for it; a link to a comment scrolls to that comment.
- **To-dos**: complete and reopen, add, assign and reschedule.
- **Card tables**: move a card to another column or straight to Done, and tick off its steps.
- **Hey!**: your notifications, with an unread count in the sidebar and desktop
  notifications. Opening an item marks it read.
- **My Assignments, My Bookmarks, Timeline, My Notes, Search**: the "me" views, across all
  your projects.
- **Several accounts**, signed in once each through Launchpad.
- **Runs in the background** (`--hidden`, or "Start at Login" in Preferences), so
  notifications arrive with the window closed.
- **Follows your desktop**: the libadwaita accent colour and, on Omarchy, the current theme.

## Keyboard

| Shortcut | Does |
| --- | --- |
| Ctrl+J or Ctrl+K | Jump to a project or page, filtered as you type |
| Alt+1 … Alt+6 | Hey!, Pings, My Assignments, My Bookmarks, Timeline, My Notes |
| Ctrl+F | Search |
| F5 | Refresh projects |
| Alt+Left | Back |
| Ctrl+Return | Post a comment |
| Ctrl+? | All shortcuts |

## Install

Skali needs gtk4 ≥ 4.18, libadwaita ≥ 1.8, webkitgtk-6.0 ≥ 2.48, blueprint-compiler and
glib2 (for `glib-compile-schemas`), plus a Rust toolchain. `install.sh` checks for these
first and names the package to install for Arch, Debian/Ubuntu or Fedora.

```
just install            # release build into ~/.local (or: sh install.sh)
PREFIX=/usr just install  # system-wide, run as root
just uninstall          # remove it again; add --purge to drop your cache, settings and sign-in
```

From a checkout, `just run` builds and starts it without installing anything.

## Privacy

Skali talks only to Basecamp (`3.basecampapi.com`, Launchpad, and Basecamp's own asset
hosts for images and avatars). Sign-in tokens are kept in your desktop keyring (the Secret
Service). Cached projects and the event-feed position are kept in `~/.local/share/skali`,
and avatars in `~/.cache/skali/avatars`.
Rich text is shown with JavaScript off and nothing loaded from the network. Links open in
your browser, or in Skali when they point to something Skali can show.

Sign-in uses Skali's own Launchpad integration, whose client id and secret are public, like
those of 37signals' own CLI. To use your own integration, set `SKALI_OAUTH_CLIENT_ID` and
`SKALI_OAUTH_CLIENT_SECRET` at build or run time. Its redirect URI must be
`http://127.0.0.1:8976/callback`.

## Development

```
just run      # cargo run -p skali
just check    # clippy -D warnings + rustfmt --check + cargo test (CI runs the same)
```

`SKALI_LOG=debug` turns up logging on stderr. `~/.cache/skali/skali.log` always has
debug-level logs. `CLAUDE.md` describes the layout, the threading rule and the other
conventions. `PLAN.md` is the plan and what's left of it.

## License

GPL-3.0-or-later.
