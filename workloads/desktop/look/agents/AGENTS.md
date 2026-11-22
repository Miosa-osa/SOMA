# Using this MIOSA computer (for agents)

You are on a MIOSA desktop: a full Linux computer (Ubuntu 24.04) with a screen, a mouse and a keyboard.
A person may be watching the same screen.
Prefer the fastest reliable path below.

## The fastest path, in order

1. **Shell first.** If a task can be done with a command, do it in a shell. It is faster and leaves less to go wrong than clicking.
2. **Browser tasks: drive Chrome through the driver's browser tools**, not by clicking pixels. They act on page elements by reference.
3. **Desktop apps: use the accessibility tree**, then click by element. Use screenshots and coordinates only when nothing else works.

## The screen

- Display `:0`, user `ubuntu`, home `/home/ubuntu`. Run GUI commands with `DISPLAY=:0`.
- Screen size: `xdpyinfo | grep dimensions`. The size follows the viewer's window, so read it before using coordinates.
- Screenshot (whole screen, PNG): `scrot -o /tmp/screen.png`. The Print key saves one to `~/Pictures/Screenshots`.

## Computer use with cua-driver (preferred)

The Cua driver runs in this session. Its tools:

| Need | Tool |
|---|---|
| See the whole screen | `get_desktop_state` |
| Windows and apps | `list_windows`, `list_apps`, `launch_app`, `bring_to_front` |
| A window's controls, as text | `get_window_state` (accessibility tree as Markdown, with element tokens) |
| Act | `click`, `double_click`, `right_click`, `drag`, `scroll`, `type_text`, `set_value`, `press_key`, `hotkey` |
| Check or zoom | `verify_state`, `zoom` (a cropped close-up of a region) |
| Several actions in one call | `run_steps` (stops at the first failure; add `observe` to see what changed) |
| Browser | `browser_prepare`, then `browser_navigate`, `browser_click`, `browser_type`, `get_browser_state` |

Call it from a shell with `cua-driver call <tool> '<json>'`, or connect it as an MCP server with `cua-driver mcp`.
`cua-driver describe <tool>` shows a tool's arguments, and `cua-driver call get_guide '{}'` returns the driver's own guide.

Without the driver, `xdotool` works on display `:0`:
`xdotool mousemove 640 400 click 1`, `xdotool type 'hello'`, `xdotool key ctrl+l`, `xdotool search --name Chrome`.

## Apps on this computer

- Installed: Google Chrome (`google-chrome <url>`), Files (`pcmanfm`), Terminal (`xfce4-terminal`), text editor (`mousepad`), images (`ristretto`), PDF (`mupdf`), calculator (`galculator`).
- Install more with one command, no password: `miosa-apps list`, then `miosa-apps install <id>`.
  Examples: `docker`, `vscode`, `nodejs`, `python`, `gh`, `claude-code`, `firefox`, `libreoffice`, `gimp`, `vlc`.
  The person can do the same from the **Apps** window.
- Ordinary `sudo` asks for a password and there is none. Use `miosa-apps` for software, and stay in your home folder for files.

## Chrome

- Chrome opens straight to a window. No first-run or terms screens.
- Logins persist: Chrome keeps its profile in `~/.config/google-chrome` and restores the last session.
- To script Chrome directly, start it with `google-chrome --remote-debugging-port=9222` and use the DevTools protocol on `127.0.0.1:9222`.

## The desktop

- Four workspaces: Ctrl+Alt+Left/Right switches; add Shift to take the window along. `xdotool set_desktop <0-3>` from a shell.
- Search any app: Alt+Space. Terminal: Ctrl+Alt+T. Snap a window: Super+Left/Right; maximise: Super+Up.
- Wallpaper: `pcmanfm --profile miosa --set-wallpaper=<file> --wallpaper-mode=crop`, or the Wallpapers app.
- The top bar shows live CPU and memory. This computer resizes itself: processors and memory grow under load and shrink when idle, so a busy build gets room without being asked.

## Good manners on a shared screen

- Do not close windows you did not open, and say what you are about to do when the person may be watching.
- Leave downloads in `~/Downloads` and your work in `~/Documents` or a project folder.
