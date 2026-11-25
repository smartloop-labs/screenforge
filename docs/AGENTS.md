# Recording and editing with your own agent (MCP)

`sf mcp` runs a [Model Context Protocol](https://modelcontextprotocol.io) server on stdin and
stdout. With it, Claude Code, Codex, Cursor or any other MCP client can record the screen and
edit ScreenForge projects. ScreenForge runs no AI model here and spends no tokens of its own:
your agent does the thinking and calls the same editing tools the in-app assistant uses.

## Set up

From easiest to most manual:

- **In the app**: open the recording toolbar's settings menu (the gear), then the **AI agents**
  section. *Add to Claude Code* runs `claude mcp add` for you. *Add to Cursor* and *Add to VS
  Code* open those editors' install prompts. *Copy MCP command* is for any other agent.
- **Claude Code plugin**: it connects the server and adds a skill that teaches the agent how to
  record and edit. The source is in `integrations/claude-plugins` (published as
  `smartloop-labs/screenforge-plugins`). In Claude Code, run these one at a time: first add the
  marketplace,

  ```
  /plugin marketplace add smartloop-labs/screenforge-plugins
  ```

  then install the plugin:

  ```
  /plugin install screenforge@screenforge
  ```

- **Homebrew** (`brew install zot24/tap/screenforge`) and the Linux `.deb` put the tool on
  your PATH as `screenforge-cli` (`sf` is the Salesforce CLI on many machines):

  ```sh
  claude mcp add screenforge -- screenforge-cli mcp
  ```

- **By path**: the tool is `ScreenForge.app/Contents/MacOS/sf` on macOS and `sf` in the Linux
  folder. From source, use `cargo build --release -p sf-cli`, then `target/release/sf`.
  Any MCP client takes it as a stdio command:

  ```json
  { "mcpServers": { "screenforge": { "command": "/Applications/ScreenForge.app/Contents/MacOS/sf", "args": ["mcp"] } } }
  ```

The Windows build does not ship the tool yet.

## What the agent can do

- **Find or make a project**: `list_projects`, `import_video` (an .mp4/.mov file, a
  .screenstudio project or a public Loom link), and recording (below).
- **Record**: `list_capture_sources`, then `start_recording` (a display, or the area around a
  window, which follows the window as it moves; optional microphone and system audio). Then
  `pause_recording` / `resume_recording` / `add_recording_marker`, and `stop_recording`, which
  makes the project and returns its path. `cancel_recording` throws the recording away.
- **Edit**: everything the in-app assistant can do. That covers `get_project`,
  `get_transcript`, `get_activity`, `get_motion`, `view_frames`, `cut`, `set_speed`,
  `split`, zooms, masks, camera layouts, audio, on-device narration, titles and overlays,
  transitions, chapters, presets and every sidebar setting through `patch_project`.
  `transcribe` runs Whisper on this computer and returns when it is done.
- **Check and deliver**: `render_frame` returns the finished look at any time as an image.
  `export` writes the video (mp4, hevc, prores, gif, webm) and a `.details.txt` with the
  title, description and tags the agent passes (a template from the transcript otherwise).

Every editing tool takes `project`, the path of the `.screenforge` package. Tool times are
milliseconds on the edited timeline. Each edit is saved straight away, with `project.json.bak`
as before every save, and `undo_answers` reverts the agent's earlier edits.

Only in the app: Unsplash backgrounds, Jamendo music search, ElevenLabs voices and your own
cloned voice, which need a ScreenForge account. Narration through MCP uses the six on-device
Kokoro voices.

## Working alongside the app

Keep the project open in ScreenForge while your agent edits it. The editor checks
`project.json` about once a second and takes the agent's edits as one undo step (⌘Z undoes
them). If you are mid-edit at that moment, your edit wins and is saved over the agent's. The
server also reads the project again when the app has saved it, so the agent always starts
from what you see.

## Permissions (macOS)

Recording needs Screen Recording, and for clicks, keys and automatic zooms also Input
Monitoring. macOS grants these to the app that started `sf`: your terminal for Claude Code,
or the editor app for an IDE client. `list_capture_sources` reports what is granted. Without
Input Monitoring the screen is still recorded, but there are no click zooms or keystroke
labels.

## Example

> Record my screen while I demo the signup flow. … OK, stop. Cut the pauses, speed up the
> typing, zoom in on the buttons I click, add a dark gradient background and captions, and
> export it for YouTube.

## From the shell

The same pieces are plain commands too:

```sh
sf sources                                   # displays, windows, microphones, permissions
sf record --window Safari --mic default      # Enter stops; prints the project path
sf record --seconds 30 --name "Demo"
sf transcribe "…/Demo.screenforge"
sf export "…/Demo.screenforge" -o demo.mp4
```
