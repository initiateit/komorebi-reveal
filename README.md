# komorebi-reveal

![Project Screenshot](https://github.com/initiateit/komorebi-reveal/blob/main/screenshot.png?raw=true)

This is a work in progress.

Using https://github.com/YoGoUrT20/win-canvas as the base for devlopement it is an Alt+Tab replacement for Komorebi with specific integration for Komorebi/c exposed commands.

#### Featureset (more info coming soon)
   - Low memory (around 3MB) blazing fast
   - Shows DWM thumbnail of live windows, refreshes as a normal window would
   - Shows a blurred wallpaper capture as the background
   - Auto layout
   - Custom zoom level
   - Shows windows from all workspaces
   - Allows scroll of windows
   - Allows filtering of workspaces & monitors
   - Selecting a window brings your focus straight to that window, workspace and monitor.

## Build
```
cargo build --release & .\target\release\komorebi-reveal.exe
```

## How to use
   - Spawn it using Alt + Ctrl + Space
   - You can zoom in the vewport using mousewheel and Ctrl
   - Exist using escape or focus on Window
