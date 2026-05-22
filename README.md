![waylandcraft banner](/assets/title_scaled.png)

Wayland Compositor in Minecraft

[Demo video](https://youtu.be/cTkEM7b0IQw)

Now available on [Modrinth](https://modrinth.com/mod/waylandcraft)!

## LLM-Assisted Soft Fork

This is a soft fork of [WaylandCraft](https://modrinth.com/mod/waylandcraft),
developed with LLM assistance. It tracks the upstream project and adds the
following fixes and additions on top of it:

- **Xwayland support** - X11 applications run alongside native Wayland apps,
  rendered as in-world windows, with input, focus, the clipboard, and
  drag-and-drop bridged both ways between X11 and Wayland.
- **Iris shader compatibility** - in-world windows keep their true colors and
  render correctly while an Iris shaderpack is active.
- **Window buffer and framebuffer lifecycle fix** - corrects window flicker and
  resize handling so windows render stably.
- **Closing in-world windows** - windows can be closed from the window manager
  screen.

The vendored Iris API jar (`libs/iris.jar`) is Iris `1.10.9+mc26.1.1`.

This fork's code is LLM-assisted, so it is kept as a separate fork and is not
submitted upstream, in keeping with the original project's contribution policy
below.

## System dependencies
- OS: Linux
- Minecraft 26.1.2
- Fabric mod loader
- xkbcommon library 1.11.0
- xkbcommon tools (xkbcli)

Additionally recommended:
- Prism Launcher
- Sodium

## Important notes for installing / using!!!
1. Do not use a Minecraft launcher packaged as a flatpak! You won't be able to use your apps.
2. For nvidia: Set the `__GL_THREADED_OPTIMIZATIONS` environment variable to `0` in your launcher.
3. If you have weird graphics glitches on nvidia, enable the "Improved Transparency" option in the video settings.

## Keybinds

| Key | Action |
| --- | --- |
| `V` | Open the app launcher |
| `B` | Open the window manager screen |
| `G` | Capture the keyboard so keystrokes reach the focused window |
| `Alt + Q` | Toggle hard keyboard capture - also forwards `Esc` and grabs the mouse for 3D apps; press again to release |

`V`, `B`, and `G` are rebindable in Minecraft's Controls settings.

While grabbing a window - press and hold the "Grab" button in the window
manager screen - these controls move and resize it:

- **scroll** - move toward or away
- **`Alt` + scroll** - move up and down
- **`Shift` + scroll** - move left and right
- **`Ctrl` + drag** - resize the window

## Frequently Asked Questions
### How do I use this thing?
Download the mod from the releases section, install Minecraft Fabric for 26.1.2 and drag the jar file in your mods folder.
Look at your keybind settings. By default `V` opens the app launcher, `G` enables keyboard capture allowing you to type in
the windows, `B` opens the window manager screen.

### How can I press Escape in the windows?
Instead of using `G` to capture the keyboard, use `ALT+Q` instead. The only way to turn it off is to press `ALT-Q` again,
so the `ESC` key is forwarded to the application.

### How to do the relative mouse movement thing for 3D games?
Move your mouse over the window, then activate the hard keyboard capture mode. (`ALT-Q`)
Exiting the hard keyboard capture mode releases the mouse.

### Will there be multiplayer support?
Multiplayer support would require video streaming, a bunch of networking code and a rewrite of input handling,
so it's not really planned right now.

### But can I use it on a server though?
You can, but because it's a client-side mod, other players won't see your windows or be able to interact with them.
Also you will not receive the windows as items. To spawn a window in the world, go into the wm screen (default bind `B`)
and then press and hold the "Grab" button.

### Does this work in VR?
Depending on your VR mod, you can probably get the windows to display fine but you probably won't be able to interact with
the windows using your controller. Soooo, kinda.

### Does this work with shaders?
The windows are rendered into the world by themselves (not like blocks or entities) so a lot of shaders will break the functionality.

## Building and Running
You need a Rust development environment and a Java 25 SDK.
```sh
./build.sh #all arguments are passed to cargo build
```

The final jar file will be in `build/libs`, or run `./gradlew runClient`
for a development environment


## Images
![soft-fork screenshot](/assets/soft-fork.png)
![screenshot](/assets/screenshot.png)

## Disclaimer
This compositor still has lots of issues and bugs. Use it at your own risk or whatever.

## Contribution Policy
All contributions have to be made an accordance with the GPLv3 license (see `LICENSE`).
Waylandcraft has some important policy around LLMs and generative AI, mostly because of code and contribution quality as well as some ethical and copyright concerns.
Mergeable contributions made to the repository in the form of pull requests need to be made **without major usage** of LLMs.

If you feel as though you have something worthwhile to contribute which was made using LLMs **please disclose it** and file it as a **draft** pull request instead.
It will probably have to be more closely examined or even entirely rewritten by a human programmer, which can then be (re-)submitted as a normal pull request.
