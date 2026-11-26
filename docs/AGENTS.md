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
- **Describe the video and let ScreenForge make it**: `ask_editor` hands a whole job to
  ScreenForge's own AI editor in plain words ("a 30 s vertical TikTok of this, narrated by
  Eric, a mountain photo behind, music from my favourites"). The editor looks through the
  recording, cuts, zooms, narrates and styles it, and answers with a short report. The
  transcript, the activity log and the frames stay inside ScreenForge, so your agent spends
  far fewer tokens than editing call by call. Later calls continue the same chat. It runs on
  your ScreenForge plan, as the agent panel in the app does; the other tools need no plan.
- **Several shots, and a promo-style video**: `join_project` adds another project or a video
  file to the end of the project. With a title card as the hook, short headlines over the
  footage timed to the narration, highlight captions, transitions, music and a closing card
  with the call to action, that makes a vertical feature short of the kind a motion-graphics
  tool produces, and it stays a ScreenForge project you can change by hand.
- **Templates, for a video without a recording**: `list_templates` shows the built-in ones
  (feature short, how-to in steps, announcement, top three, quick tip, did you know, before
  and after, testimonial, sale) and how to give a card. A card given as an object (a list, a
  checklist, a figure, a quote, a comparison, a price, a mock window) is laid out as separate
  elements, each shape an overlay and each piece of text a title, so every one can be
  selected, moved, retyped or deleted in the app; a card given as SVG is one image.
  `new_from_template` makes the video from what the agent writes: a hook, up to eight beats
  (a two-line headline, a spoken sentence and a card drawn as SVG) and an end card with a
  button and a web address. Each part lasts as long as its narration, which is read aloud and
  captioned. The result is an ordinary 1080×1920 project: every title, card and narration
  take can be changed afterwards, by the agent or by hand in ScreenForge.
  In the app the same templates are under *Templates* in the recent-projects panel: choosing
  one makes the project with stand-in titles and cards to change by hand. From the shell:
  `sf template feature-short` (add `--spec file.json` for your own words and cards).
- **Artwork the agent draws**: `add_overlay` takes `svg` in place of a file, so an agent can
  put an illustration, a diagram or a card over any video.
- **TikTok, Reels and Shorts**: `set_format` makes the video 1080×1920 and keeps the text
  inside the platform's safe area, the part of the frame the app's own caption, buttons and
  tabs leave clear. Captions sit just above the covered bottom, titles are moved in (and
  made smaller when a line is too long), and titles added later stay inside too. `export`
  then writes 1080×1920 by default.
- **Open it in the app**: `open_project` opens the project in ScreenForge, where you watch
  it and keep editing; the agent's later edits show up there.
- **Check and deliver**: `render_frame` returns the finished look at any time as an image.
  `export` writes the video (mp4, hevc, prores, gif, webm) and a `.details.txt` with the
  title, description and tags the agent passes (a template from the transcript otherwise).
  It adds the credit lines that Creative Commons music and Unsplash photos ask for under the
  description and returns them as `credits`, ready for the caption.

Every editing tool takes `project`, the path of the `.screenforge` package. Tool times are
milliseconds on the edited timeline. Each edit is saved straight away, with `project.json.bak`
as before every save, and `undo_answers` reverts the agent's earlier edits.

The tools are the ones the app's agent panel uses, so an agent can do the same from either:
Unsplash backgrounds (`search_photos`, `use_photo`), Jamendo music and your favourite tracks
(`search_music`, `use_music`, `favorite_music`), and narration with the on-device voices,
the ElevenLabs voices (studio quality, paid in credits) and the voices you made of your own.
Unsplash, Jamendo and ElevenLabs go through your ScreenForge account, as in the app.

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

> Make a 20 second feature short about our new export presets from the feature-short
> template, narrated by Eric, with music from my favourites.

> Make a 25 second TikTok of my last recording: vertical, narrated by Eric, a mountain photo
> behind it and music from my favourites. Export it and give me the caption with the credits.

## From the shell

The same pieces are plain commands too:

```sh
sf sources                                   # displays, windows, microphones, permissions
sf record --window Safari --mic default      # Enter stops; prints the project path
sf record --seconds 30 --name "Demo"
sf transcribe "…/Demo.screenforge"
sf export "…/Demo.screenforge" -o demo.mp4
```
