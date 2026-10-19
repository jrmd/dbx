# Themes

Open **Settings → Appearance**. Palettes are in the **Theme** group at the top.

- Choose DBX, Ocean, Forest, Plum, Sand, Graphite, Dracula, Nord, Solarized, or Catppuccin. Each preset has light and dark variants; System follows the OS appearance.
- **Your themes** lists every imported or edited theme. The dashed **Import…** tile at the end adds a VS Code `.json`/`.jsonc`, Zed theme family, or DBX theme; importing a theme with the same name updates it instead of duplicating it. Selecting a theme made for one appearance switches DBX to light or dark to match; its thumbnail always previews that variant. Hovering a theme reveals a × in its corner (red when targeted) that removes it after confirmation; removing the active theme returns to the preset.
- Click **Edit…** in the Colours row, then search the colour list to edit any interface, material, grid, or SQL syntax colour. Use the colour picker or enter `#RGB`, `#RGBA`, `#RRGGBB`, or `#RRGGBBAA`, then **Apply** (or Enter). Alpha is the last component.
- Changes apply immediately and persist. Edit the light and dark variants independently. **Reset** restores that token from the base preset. Editing a preset saves a new theme (“Custom Forest”, “Custom Forest 2”, …) under Your themes, so earlier edits are never overwritten.
- **Export…** saves the selected theme as a portable DBX JSON containing every colour in both appearances. Files are processed locally.

The familiar presets adapt the original [Dracula](https://draculatheme.com/spec),
[Nord](https://www.nordtheme.com/docs/colors-and-palettes/),
[Solarized](https://ethanschoonover.com/solarized/), and
[Catppuccin](https://catppuccin.com/palette/) palettes to DBX's controls and SQL roles.
Catppuccin uses Latte in light appearance and Mocha in dark appearance. Light
Dracula and Nord are DBX adaptations. Contrast correction can adjust muted or
syntax colours to keep them readable.

## Zed imports

Import a self-contained Zed theme family JSON/JSONC using **Import…**. Every
entry in its `themes` array is added to **Your themes**, and the first is applied.
A single theme object with `name`, `appearance`, and `style` is also accepted.
Appearance must be `light` or `dark`; each imported variant retains its own name.
The full file is validated before any saved themes change.

The importer maps editor and panel surfaces, text, borders, controls, selections,
status colours, and common SQL syntax captures. Missing roles use DBX fallbacks;
alpha channels and contrast correction work as for VS Code imports. Font styles,
terminal colours, icon definitions, and extension packages are not imported.
The mapping follows [Zed's theme family format](https://zed.dev/docs/extensions/themes).

## VS Code imports

Import a self-contained VS Code colour theme `.json` or `.jsonc` (up to 2 MB).
Comments, trailing commas, short hex colours, and alpha channels are accepted.
The importer maps editor surfaces, text, sidebar, panels, buttons, focus, selection,
status colours, and common `tokenColors` scopes for SQL keywords, strings,
comments, numbers, parameters, identifiers, and types. Missing roles use DBX
fallbacks or surfaces derived from the imported editor background.

Each syntax role takes the most generic matching scope (`keyword`, `string`,
`comment`, …) rather than whichever language-specific override appears last,
and selectors with descendant scopes are ignored. Translucent text colours are
flattened onto the editor background. Text and status colours are raised to
4.5:1 contrast; input borders, focus rings, and syntax colours to 3:1, so faint VS Code colours such as
line numbers stay readable as DBX labels and borderless inputs keep an edge.
The same floor applies when a saved theme loads.

Imports select their declared light/dark appearance. When `type` is absent,
DBX infers appearance from `editor.background`. The other appearance uses the
DBX palette until you customise it. Grid selections use washes derived from
VS Code's list selection so DBX's semantic cell text remains visible.

VS Code and DBX expose different UI and syntax roles, so imports are an
approximation rather than a pixel-identical VS Code renderer. Semantic token
rules, font styles, extension packaging (`.vsix` / `package.json`), and external
`include` / token files are not imported. For a theme with external files, run
**Developer: Generate Color Theme From Current Settings** in VS Code and import
the generated JSON. Invalid files leave the active and saved theme unchanged.

The mapping follows VS Code's [theme colour reference](https://code.visualstudio.com/api/references/theme-color)
and [colour theme format](https://code.visualstudio.com/api/extension-guides/color-theme).

## DBX JSON

```json
{
  "dbx_theme": 1,
  "name": "My theme",
  "light": { "canvas": "#f7f9fc", "grid_active": "#b9d6fa" },
  "dark": { "canvas": "#0a0c10", "grid_active": "#19355a" }
}
```

Partial DBX imports use the DBX preset for omitted colours. Export from Settings
for the complete, current token list. Unknown tokens and invalid hex values are
rejected. Reduced transparency keeps window/panel materials opaque, including
imported colours. Custom colours are used as supplied; use the grid and SQL
preview to check readability.
