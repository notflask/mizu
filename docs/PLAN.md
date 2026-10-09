# mizu: Implementierungsplan

> **mizu** (水, Wasser) ist ein sehr minimalistischer PDF-Viewer fürs Studium.
> Er kann Vim-Keybindings, einen Dunkelmodus für das PDF selbst, Zoom ohne Ruckeln
> und Malen mit Stift und Radierer inklusive Speichern.
> Plattformen: **Linux (Hauptziel: NixOS, Wayland, Niri/Hyprland)**, Windows, macOS.
>
> **mizu muss sehr gut optimiert sein.** Performance ist eine harte Anforderung und kein
> Nice-to-have. Abschnitt 19 ist genauso verbindlich wie die Features.

Dieses Dokument ist die verbindliche Spezifikation für die Umsetzung. Es richtet sich an
einen Coding-Agenten bzw. Entwickler, der das Projekt Meilenstein für Meilenstein baut.

---

## 0. Hinweise für den umsetzenden Agenten

- **Arbeite strikt Meilenstein für Meilenstein** (Abschnitt 17). Am Ende jedes Meilensteins:
  `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. Erst wenn alles grün ist,
  committen. Ein Commit pro abgeschlossenem Teilschritt.
- **Crate-APIs vor Benutzung auf docs.rs prüfen.** Versionen ändern sich, und Code „aus dem
  Gedächtnis“ ist oft veraltet. Das gilt besonders für `winit` (ab 0.30 mit `ApplicationHandler`-API),
  `wgpu` und `mupdf`. Nimm jeweils die aktuelle stabile Version und pinne sie über `Cargo.lock`
  (wird committet).
- **`glyphon` muss zur `wgpu`-Version passen.** Wähle die `wgpu`-Version, die die aktuelle
  `glyphon`-Version verlangt.
- Fehlerbehandlung mit `anyhow` (App) bzw. `thiserror` (Module mit eigenen Fehlertypen).
  Bei Benutzereingaben, Dateien und PDFs **niemals `unwrap()`**: Fehler kommen als Meldung in die Statuszeile.
- Logging über `log` + `env_logger` (`RUST_LOG=mizu=debug`).
- Keine zusätzlichen Features erfinden. Was nicht in diesem Plan steht, wird nicht gebaut
  (siehe Nicht-Ziele, Abschnitt 2).
- Code und Kommentare auf Englisch, Benutzertexte (Statuszeile, Fehlermeldungen) auf Englisch.
- **Performance hat höchste Priorität (Abschnitt 19).** Bei jeder Designentscheidung gewinnt im
  Zweifel die schnellere und sparsamere Lösung. Ein Meilenstein ist erst fertig, wenn seine
  Performance-Ziele **gemessen** (nicht geschätzt) erreicht sind. Die Messwerte kommen in
  `docs/TESTING.md`. Kein Meilenstein darf die Werte eines früheren verschlechtern.
- **Alle Meilensteine M0–M9 sind Pflicht.** Nichts in diesem Plan ist optional, außer es ist
  ausdrücklich als „nur wenn möglich“ markiert, mit Begründung.

---

## 1. Entscheidungen (mit dem Nutzer abgestimmt)

| Thema | Entscheidung |
|---|---|
| Sprache | **Rust** (stable, Edition 2021 oder neuer) |
| PDF-Engine | **MuPDF** über das Crate `mupdf` (mupdf-rs, baut MuPDF statisch aus dem Quellcode) |
| Lizenz | **AGPL-3.0-or-later** (wegen MuPDF); `LICENSE`-Datei anlegen |
| Fenster/Input | `winit` |
| Grafik | `wgpu` (Vulkan/Metal/DX12, GL als Fallback) |
| UI-Stil | Wie **sioyek**: keine Toolbar und keine Menüs, nur Seiten, eine einzeilige Statuszeile und eine `:`-Kommandozeile |
| Dunkelmodus | Färbt **alles** im PDF um, auch Bilder. Der Farbton bleibt erhalten, die Helligkeit wird invertiert |
| Dunkelmodus-Farben | Hintergrund **komplett schwarz `#000000`**, Text **weiß `#ffffff`**. Auch Fensterhintergrund, Seitenabstände und Statuszeile sind im Dunkelmodus schwarz |
| Performance | **Höchste Priorität**: flüssig bei jeder Bildwiederholrate, 0 % CPU im Leerlauf, sparsamer Speicher, kleine Binary (Abschnitt 19) |
| Zeichnen | Nur **Stift** (Farbe, Dicke) und **Radierer**, mit **Undo/Redo**. Kein Textmarker, keine Textnotizen, keine Formen |
| Eingabegerät | Maus und **Stift mit Druckstufen** auf allen Plattformen, inklusive Radier-Ende des Stifts (M8). Touchpad-Pinch-Zoom überall, auch auf Wayland |
| Speichern | **Wie Vim**: `:w` überschreibt die Originaldatei, kein Autosave. Striche werden als echte PDF-Ink-Annotationen gespeichert |
| Ansicht | Ein Dokument pro Fenster, durchgehend vertikales Scrollen, **keine** Doppelseitenansicht |
| Icon | Seite mit Eselsohr, durch die eine Welle läuft: oben hell, unten dunkel („eingetaucht“) |

---

## 2. Nicht-Ziele

Diese Features werden **nicht** gebaut: Textauswahl und Kopieren, Text-Highlight-Annotationen,
Textnotizen, Formen, Doppelseitenansicht, Tabs, Präsentationsmodus, Formulare ausfüllen,
Drucken, eingebauter Dateidialog, Thumbnails-Seitenleiste, Plugin-System, EPUB und andere Formate.

---

## 3. Projektstruktur

```
mizu/
├── Cargo.toml
├── Cargo.lock
├── LICENSE                       # AGPL-3.0-or-later
├── README.md                     # Kurzbeschreibung, Installation, Keybindings
├── flake.nix / flake.lock        # NixOS: devShell + Paket
├── build.rs                      # Windows: Icon in .exe einbetten
├── docs/PLAN.md
├── assets/
│   ├── fonts/                    # eine kleine OFL-Schrift für die Statuszeile (z. B. Inter oder JetBrains Mono)
│   └── icons/
│       ├── src/                  # SVG-Quellen (Hand gepflegt)
│       └── generated/            # erzeugte PNG/ICO/ICNS (committet)
├── packaging/
│   ├── linux/io.github.notflask.Mizu.desktop
│   ├── macos/Info.plist
│   └── windows/mizu.rc (falls nötig)
├── xtask/                        # Hilfsprogramm: Icons generieren (`cargo xtask icons`)
├── tests/fixtures/               # kleine Test-PDFs (inkl. rotierte Seite, gemischte Seitengrößen)
└── src/
    ├── main.rs                   # CLI-Parsing, Logger, Event-Loop starten
    ├── app.rs                    # winit ApplicationHandler, verbindet alle Teile
    ├── config.rs                 # config.toml laden, Defaults
    ├── session.rs                # letzte Position pro Datei (XDG state dir)
    ├── doc/
    │   ├── mod.rs                # Document-Metadaten (Seitenanzahl, Seitengrößen), Öffnen
    │   ├── worker.rs             # Render-Worker-Pool, Job-Queue, Abbruch veralteter Jobs
    │   ├── annots.rs             # mizu-Striche aus PDF laden / in PDF speichern
    │   ├── ffi_ext.rs            # fehlende MuPDF-Funktionen über mupdf-sys (unsafe, gekapselt)
    │   ├── search.rs
    │   ├── outline.rs
    │   └── links.rs
    ├── view/
    │   ├── layout.rs             # Seiten untereinander anordnen (Dokumentkoordinaten)
    │   └── camera.rs             # Offset + Zoom, Umrechnung Bildschirm <-> Dokument <-> Seite
    ├── render/
    │   ├── gpu.rs                # wgpu Device/Surface/MSAA-Target
    │   ├── tiles.rs              # Tile-Textur-Cache (LRU, Speicherbudget)
    │   ├── pages.rs + pages.wgsl # Seiten-Quads zeichnen, Recolor im Shader
    │   ├── strokes.rs + strokes.wgsl # Strich-Geometrie (lyon), Recolor im Shader
    │   ├── recolor.wgsl          # gemeinsame Recolor-Funktion (per include/concat)
    │   └── ui.rs                 # Statuszeile, Kommandozeile, Overlays (glyphon)
    ├── input/
    │   ├── keys.rs               # Parser für "<C-d>", "gg", "<Space>" usw.
    │   ├── keymap.rs             # Modus-abhängige Keymaps, Präfix-Sequenzen, Counts
    │   ├── actions.rs            # enum Action
    │   └── command.rs            # ":"-Kommandos parsen, Tab-Completion für Pfade
    ├── ink/
    │   ├── stroke.rs             # Stroke-Datenmodell
    │   ├── smooth.rs             # Glättung + Vereinfachung (RDP)
    │   ├── eraser.rs             # Hit-Test
    │   └── history.rs            # Undo/Redo
    ├── platform/                 # was winit (noch) nicht liefert
    │   ├── mod.rs                # gemeinsames Interface: Pinch- und Stift-Events
    │   ├── wayland.rs            # pointer-gestures-v1 (Pinch) + tablet-v2 (Stift)
    │   ├── x11.rs                # XInput 2.4: Gesten + Stift-Druck
    │   ├── windows.rs            # WM_POINTER: Stift-Druck + Radier-Ende
    │   └── macos.rs              # NSEvent-Monitor: Stift-Druck + Radier-Ende
    ├── perf.rs                   # Frame-/Tile-Statistiken (`--stats`), Profiling-Hooks
    └── watch.rs                  # Datei-Überwachung für Auto-Reload
benches/                          # criterion-Benchmarks (siehe 19.6)
```

App-ID (Wayland `app_id`, Desktop-Datei, macOS Bundle-ID): **`io.github.notflask.Mizu`**.

---

## 4. Abhängigkeiten (Richtwerte, Versionen beim Start prüfen)

| Crate | Zweck |
|---|---|
| `mupdf` (+ `mupdf-sys` für Lücken) | PDF öffnen, rendern, Annotationen, Suche, Outline, Links |
| `winit` | Fenster, Tastatur, Maus, Touch, DnD |
| `wgpu` | Rendering |
| `glyphon` | Text für Statuszeile und Overlays |
| `lyon_tessellation` | Strich-Tessellierung |
| `bytemuck` | Vertex-Daten |
| `crossbeam-channel` | Job- und Ergebnis-Queues |
| `notify` + `notify-debouncer-mini` | Auto-Reload |
| `serde`, `toml` | Config |
| `serde_json` | Session-Datei |
| `directories` | Config- und State-Pfade plattformgerecht |
| `lexopt` oder `clap` (derive) | CLI |
| `anyhow`, `thiserror`, `log`, `env_logger` | Infrastruktur |
| `open` | externe Links öffnen |
| `uuid` | IDs für Striche (`/NM`) |
| `tempfile` | atomisches Speichern |
| `pollster` | wgpu-Init blockierend |
| Dev/xtask: `resvg`, `tiny-skia`, `usvg`, `ico`, `icns` | Icon-Generierung |
| Build (Windows): `embed-resource` oder `winresource` | Icon in .exe |
| `raw-window-handle` | an die nativen Handles kommen (für `platform/`) |
| Linux: `wayland-client`, `wayland-protocols` (Features `unstable`, `client`) | Pinch- und Tablet-Protokolle |
| Linux: `x11rb` (Feature `xinput`) | X11-Fallback für Gesten und Stift |
| Windows: `windows` (nur benötigte Features) | `GetPointerPenInfo`, Fenster-Subclassing |
| macOS: `objc2`, `objc2-app-kit`, `objc2-foundation` | NSEvent-Monitor für Stift |
| `profiling` (+ Backend `tracy`, nur hinter dem Cargo-Feature `profile`) | Profiling ohne Kosten im Release-Build |
| Dev: `criterion` | Benchmarks |

---

## 5. Koordinatensysteme und Layout

Drei Räume, sauber getrennt in `view/camera.rs`:

1. **Seitenraum** (pro Seite): Einheit = PDF-Punkt (1/72 Zoll), Ursprung oben links, y nach unten.
   Das ist MuPDFs „fitz space“ nach Anwendung der Seitenrotation, also das, was
   `page.bounds()` liefert. **Alle Striche werden im Seitenraum gespeichert.**
2. **Dokumentraum**: Alle Seiten liegen untereinander, horizontal zentriert auf die breiteste Seite,
   mit `page_gap = 8 pt` Abstand. `layout.rs` hält pro Seite `origin_y` und `size`.
3. **Bildschirmraum**: physische Pixel.
   `screen = (doc - camera.offset) * camera.zoom * scale_factor`.

Kamera:
- `zoom` = logische Pixel pro Punkt. Erlaubt sind 0.1 bis 32.0.
- **Zoom zum Cursor**: Der Dokumentpunkt unter dem Cursor bleibt beim Zoomen fix (Unit-Test!).
- Zoommodi: `FitWidth` (Standard), `FitPage`, `Free`. In `FitWidth` und `FitPage` wird der Zoom
  bei Fenstergrößenänderung neu berechnet. Das passiert unter Niri/Hyprland oft, deshalb wird das
  Nachrendern entprellt (siehe 6).
- „Aktuelle Seite“ = die Seite, die die vertikale Bildschirmmitte schneidet.
- Fensterränder clampen: Man kann nicht endlos über Anfang und Ende hinausscrollen. Horizontal ist
  nur Scrollen möglich, wenn der Inhalt breiter als das Fenster ist; sonst wird zentriert.

---

## 6. Rendering und Zoom (Kernqualität!)

**Ziel:** Text ist bei jedem Zoom gestochen scharf, und Scrollen und Zoomen laufen immer in
Bildwiederholrate. Wenn noch keine scharfen Kacheln da sind, wird kurz etwas unscharf angezeigt,
aber das Bild bleibt nie leer und ruckelt nie.

### 6.1 Worker-Pool
- MuPDF-Objekte sind nicht `Send`. Deshalb öffnet **jeder Worker-Thread das Dokument selbst**.
  Es gibt `N = clamp(cores - 1, 1, 4)` Worker, und der Hauptthread hält nur Metadaten.
- Jobs: `RenderTile { page, scale, tile_x, tile_y, generation }`, `RenderThumb { page }`,
  `Search { ... }`. Ergebnisse gehen über einen Channel zurück. Danach weckt
  `EventLoopProxy::send_event` die Event-Loop.
- **Abbruch**: Bei jeder Kamera-Änderung zählt `generation` hoch. Worker verwerfen Jobs mit
  veralteter Generation, bevor sie rendern. Die Queue ist eine Prioritätsqueue: sichtbare
  Kacheln zuerst (von der Bildschirmmitte nach außen), dann Prefetch (je ein Bildschirm darüber
  und darunter).
- Pro Worker wird eine Display-List pro Seite gecacht (`Page::to_display_list`, LRU über
  ~8 Seiten). Kacheln rendern dann aus der Display-List mit Clip auf das Kachel-Rechteck.

### 6.2 Kacheln
- Kachelgröße: 512×512 physische Pixel.
- `render_scale = zoom * scale_factor`. Gerendert wird mit **exakt** diesem Wert, damit der Text
  nicht durch Resampling weich wird. Neue Jobs erst, wenn sich der Zoom **120 ms** nicht mehr
  geändert hat.
- **Während des Zoomens** werden die vorhandenen Kacheln der nächstliegenden Skala gestreckt bzw.
  gestaucht angezeigt (lineares Filtern).
- **Fallback-Ebene**: Pro Seite gibt es eine kleine Vorschau-Textur (passt in ein Feld von
  256×256 px). So ist beim schnellen Scrollen (`G`, `gg`) sofort etwas zu sehen.
  - Die Vorschauen werden mit niedrigster Priorität im Hintergrund gerendert, von der aktuellen
    Seite nach außen.
  - Für sie gilt ein eigenes Budget von **48 MB**. Bei großen Dokumenten werden die Vorschauen
    behalten, die der aktuellen Seite am nächsten sind.
- Cache: Schlüssel `(page, scale_bits, tx, ty)`, LRU mit GPU-Speicherbudget **256 MB**
  (konfigurierbar).
- **Texturen als Arrays**: Alle Kacheln liegen in wenigen großen `texture_2d_array`s mit festen
  512×512-Slots. Pro Array gibt es höchstens so viele Layer, wie `max_texture_array_layers`
  erlaubt, und es werden so viele Arrays angelegt, wie das Budget hergibt. Vorschauen liegen
  ebenso in einem eigenen Array mit 256×256-Slots.
  - Eine Kachel = Instanz-Daten `(rect, slot, uv)`.
  - Alle sichtbaren Kacheln werden mit **einem instanzierten Draw-Call pro Array** gezeichnet,
    nicht mit einem Draw-Call und einer Bind-Group pro Kachel.
  - Freigewordene Slots werden wiederverwendet. Zur Laufzeit wird nie eine Textur neu angelegt
    oder freigegeben.
- Pixmap: MuPDF rendert direkt in einen **RGBA-Pixmap** (mit Alpha-Kanal, vorher mit opakem Weiß
  gefüllt). So entfällt die Konvertierung von RGB nach RGBA. Pixmap-Puffer kommen aus einem Pool
  pro Worker und werden wiederverwendet. Hochgeladen wird als `Rgba8UnormSrgb`; der Shader sieht
  also lineare Werte.
- **Upload-Budget**: Pro Frame werden höchstens 8 Kacheln (≈ 8 MB) per `queue.write_texture`
  hochgeladen. Der Rest kommt im nächsten Frame dran (`request_redraw`). So gibt es keine
  Frame-Spitzen, wenn viele Kacheln auf einmal fertig werden.
- Anti-Aliasing: MuPDF-Default (8 Bit). Kein eigenes Gamma-Gefummel.

### 6.3 Frame-Ablauf
- `ControlFlow::Wait`. Neu gezeichnet wird nur bei Input, Animation, eintreffender Kachel oder Resize.
- Reihenfolge pro Frame:
  1. Hintergrund (über das Clear der Render-Pass, kein extra Draw), im Dunkelmodus `#000000`
  2. für jede sichtbare Seite: zuerst die Vorschau, darüber die besten verfügbaren Kacheln
  3. die Striche der Seite
  4. eventuelle Suchtreffer
  5. die Statuszeile
- MSAA 4× für die Striche. Die Seitentexturen werden ohne MSAA in dasselbe Target gezeichnet;
  dafür reicht ein MSAA-Target mit Resolve.
- Present-Mode: `AutoVsync`. Pro Frame gibt es **eine** Render-Pass. Seiten, Striche, Treffer und UI
  laufen über vorab erstellte Pipelines mit persistenten, wachsenden Buffern; pro Frame wird
  nichts neu alloziert.
- Nur Sichtbares wird gezeichnet: Seiten, Kacheln, Striche und Treffer werden per Bounding-Box
  gegen den Viewport gecullt.

---

## 7. Dunkelmodus (Recolor)

- Er gilt für den Seiteninhalt (inklusive Bildern und Strichen). Im Dunkelmodus sind außerdem
  **Fensterhintergrund, Seitenabstände und Statuszeile komplett schwarz `#000000`**. Die Schrift
  der Statuszeile ist weiß, Nebeninformationen sind grau (`#8a8a8a`).
- Es gibt keine Trennlinie zwischen den Seiten (alles schwarz). Optional kann man in der Config
  eine Farbe dafür setzen (`dark.separator`); standardmäßig ist sie aus.
- Er ist eine reine Darstellungssache. Gespeicherte Strichfarben sind immer die Originalfarben.
- Die Funktion `recolor(rgb_linear) -> rgb_linear` wird im Seiten- **und** im Strich-Shader benutzt:

```
// in OKLab
lab      = oklab_from_linear_srgb(c)
fg, bg   = oklab der konfigurierten Farben (Uniforms; bg = Seitenhintergrund im Dunkelmodus)
t        = lab.L                       // 1 = weiß, 0 = schwarz
L'       = mix(fg.L, bg.L, t)          // weiß -> bg, schwarz -> fg
tint     = mix(fg.ab, bg.ab, t)        // neutrale Töne -> exakt Verlauf fg..bg
ab'      = lab.ab + tint
out      = linear_srgb_from_oklab(L', ab'), auf [0,1] clampen
```

- Eigenschaften (als CPU-Referenzimplementierung in Rust **unit-getestet**):
  Weiß wird exakt zu `bg`, Schwarz exakt zu `fg`, und gesättigtes Rot bleibt rötlich
  (Hue-Differenz < 15°).
- Standardfarben: **`bg = #000000` (komplett schwarz), `fg = #ffffff` (weiß)**, konfigurierbar.
  Bei diesen Defaults ergibt die Formel einfach `L' = 1 − L` bei unveränderten `a, b`. Test:
  `#ffffff` wird exakt zu `#000000`, `#000000` exakt zu `#ffffff`.
- Umschalten: Taste `D` bzw. `:dark`, `:light`. Der Zustand wird pro Datei in der Session
  gespeichert, global gilt der Default aus der Config.
- Umschalten ändert nur ein Uniform. Es wird **nichts** neu gerendert.

---

## 8. Eingabe und Vim-Keybindings

### 8.1 Modi
`Normal`, `Draw`, `Command` (nach `:`), `Search` (nach `/` oder `?`), `Outline` (Overlay).

### 8.2 Keymap-Engine (`input/keymap.rs`)
- Key-Notation wie in Vim: `j`, `G`, `<C-d>`, `<S-Space>`, `<Esc>`, `<Tab>`, `<CR>`, `<F5>`.
  Parser und Formatter werden unit-getestet.
- **Count-Präfix**: `5j`, `42G`, `3<C-d>`. Eine Ziffer `0` ohne vorherige Ziffer ist eine normale Taste.
- **Präfix-Sequenzen**: `gg`, `zw`, `ZZ`, `m{a-z}`, `'{a-z}`. Es gibt keinen Timeout. Ist eine
  Sequenz eindeutig ein Präfix, wird gewartet. Ist sie ungültig, wird sie verworfen.
- Die Keymap ist eine Tabelle `mode -> (sequence -> Action)`, kommt aus Defaults und wird von
  `config.toml` überschrieben bzw. erweitert. Im `Draw`-Modus gelten zusätzlich alle
  Navigationstasten aus `Normal`; Draw-spezifische Tasten haben Vorrang.
- Tasten-Wiederholung (gedrückt halten) funktioniert für Bewegungen.

### 8.3 Default-Belegung: Normal

| Taste | Aktion |
|---|---|
| `j` / `k` | 1 Schritt runter / hoch (Schritt = 60 logische px, × Count) |
| `h` / `l` | 1 Schritt links / rechts |
| `<C-d>` / `<C-u>` | halbe Fensterhöhe runter / hoch |
| `<C-f>` / `<C-b>`, `<Space>` / `<S-Space>` | ganze Fensterhöhe (minus 1 Zeile Überlappung) |
| `J` / `K` | nächste / vorherige Seite (Seitenanfang an den oberen Rand) |
| `gg` / `G` | Anfang / Ende. Mit Count: zu Seite N (`42G`, `42gg`) |
| `+` / `-` | Zoom × 1.2 / ÷ 1.2 (um die Fenstermitte) |
| `=` / `zw` | Seitenbreite |
| `zf` | ganze Seite |
| `z0` | 100 % |
| `/` / `?` | Suche vorwärts / rückwärts |
| `n` / `N` | nächster / vorheriger Treffer |
| `m{a-z}` | Marke setzen (Seite + Offset), wird in der Session gespeichert |
| `'{a-z}` und `` `{a-z} `` | zur Marke springen |
| `<C-o>` / `<C-i>` (`<Tab>`) | Sprungliste zurück / vor. Gefüllt durch `gg`, `G`, `NG`, Marken, Suche, Links, Outline |
| `o` | Inhaltsverzeichnis (Outline-Overlay) |
| `D` | Dunkelmodus an/aus |
| `i` | Zeichenmodus (Stift) |
| `u` / `<C-r>` | Undo / Redo |
| `r` | Datei neu laden |
| `:` | Kommandozeile |
| `ZZ` | wie `:x` |
| `ZQ` | wie `:q!` |
| `<Esc>` | Suchtreffer-Hervorhebung und Meldung ausblenden |

### 8.4 Default-Belegung: Draw

| Taste / Maus | Aktion |
|---|---|
| `<Esc>` | zurück zu Normal |
| Linke Maustaste ziehen | zeichnen (oder radieren, wenn der Radierer aktiv ist) |
| Rechte Maustaste ziehen | temporär radieren |
| Mittlere Maustaste ziehen | verschieben |
| `e` | Stift ↔ Radierer umschalten |
| `p` | Stift |
| `1`…`9` | Farbe aus der Palette |
| `[` / `]` | Dicke ÷ 1.25 / × 1.25 (Grenzen 0.25 bis 20 pt) |
| `u` / `<C-r>` | Undo / Redo |
| alle Navigationstasten aus Normal | wie in Normal (außer `i`, `e`, `p`, Ziffern) |

Hinweis: Ziffern sind im Draw-Modus Farben und keine Counts.

### 8.5 Maus und Touchpad (alle Modi)
- Mausrad: scrollen. `LineDelta` × Schrittweite, `PixelDelta` (Touchpad) 1:1 und weich.
- `Ctrl` + Mausrad: Zoom zum Cursor (Faktor 1.1 pro Raste, kontinuierlich bei `PixelDelta`).
- **Pinch-Geste auf dem Touchpad** zoomt zum Pinch-Mittelpunkt, und zwar **auf allen Plattformen**
  (Umsetzung in M8, Details in 9.5):
  - macOS: `winit`-Event `PinchGesture`.
  - Windows: Precision-Touchpads schicken Pinch als `Ctrl`+Mausrad, das ist damit schon abgedeckt.
    Liefert `winit` `PinchGesture` auch auf Windows, wird stattdessen das benutzt.
  - **Wayland**: Falls die verwendete `winit`-Version Pinch auf Wayland liefert, wird das benutzt.
    Sonst kommt es aus `platform/wayland.rs` über das Protokoll
    `zwp_pointer_gestures_v1` (Pinch begin/update/end).
  - X11: XInput 2.4 Gesture-Events über `platform/x11.rs`.
- Normal-Modus: Ziehen mit links oder Mitte verschiebt die Ansicht. Klick ohne Ziehen auf einen
  Link folgt dem Link.
- Drag & Drop einer PDF-Datei ins Fenster öffnet sie (wie `:e`).

### 8.6 Kommandos (`:`)

| Kommando | Wirkung |
|---|---|
| `:w` | speichern (Original überschreiben, atomar) |
| `:w <pfad>` | Kopie mit Strichen nach `<pfad>` schreiben; die aktuelle Datei bleibt dieselbe (wie Vim) |
| `:q` | beenden. Bei ungespeicherten Änderungen: `E37: No write since last change (add ! to override)` |
| `:q!` | beenden ohne Speichern |
| `:wq`, `:x` | speichern und beenden (`:x` speichert nur bei Änderungen) |
| `:e <pfad>` | andere Datei öffnen (mit derselben Dirty-Prüfung wie `:q`) |
| `:e!` | neu laden und Änderungen verwerfen |
| `:<N>` | zu Seite N |
| `:dark`, `:light` | Dunkelmodus an / aus |
| `:color #rrggbb` | Stiftfarbe setzen |
| `:width <pt>` | Stiftdicke setzen |

- `<Tab>` vervollständigt Pfade bei `:e` und `:w`; `~` wird expandiert.
- `<Up>` / `<Down>` blättern durch die Kommando-Historie (pro Sitzung).
- `<Esc>` bricht ab, `<CR>` führt aus, `<BS>` auf einer leeren Zeile beendet den Modus.
- **Fenster schließen (CloseRequested)** bei ungespeicherten Änderungen: Das erste Mal erscheint
  nur die E37-Meldung. Ein zweiter Schließversuch innerhalb von 3 s beendet das Programm.

---

## 9. Zeichnen, Radieren, Undo/Redo

### 9.1 Datenmodell (`ink/stroke.rs`)
```rust
struct Stroke {
    id: Uuid,              // landet als /NM "mizu-<uuid>" im PDF
    page: usize,
    points: Vec<[f32; 2]>, // Seitenraum, Punkte
    width: f32,            // in pt (skaliert mit Zoom), Grundbreite
    pressure: Option<Vec<f32>>, // 0..1 pro Punkt, nur bei Stift-Eingabe mit Druck
    color: [u8; 3],        // sRGB, Originalfarbe
    bbox: Rect,            // gecacht, für Culling und Radierer
}
```

### 9.2 Stift
- Ein Strich beginnt beim Drücken über einer Seite und gehört zu **dieser** Seite. Punkte
  außerhalb der Seite werden auf den Seitenrand geclamped.
- Neue Punkte werden nur übernommen, wenn sie ≥ 0,75 physische px vom letzten entfernt sind.
- Während des Zeichnens wird der Strich live tesselliert und gezeichnet. Die Latenz muss minimal
  sein: Bei jedem `CursorMoved` wird sofort neu gezeichnet.
- Beim Loslassen:
  1. leichte Glättung (Chaikin, 1 bis 2 Iterationen, oder gleitender Mittelwert über 3 Punkte;
     Endpunkte bleiben erhalten)
  2. Vereinfachung mit Ramer–Douglas–Peucker, ε = 0,1 pt (in Seitenkoordinaten, also unabhängig vom Zoom)
  3. Ein Klick ohne Bewegung ergibt einen Punkt, gezeichnet als kleiner Kreis
- Darstellung: `lyon` StrokeTessellator mit runden Caps und Joins und Breite = `width`.
  Vertex-Buffer werden pro Strich gecacht (Seitenraum, die Transformation macht der Shader).
- Cursor im Draw-Modus: Kreis mit dem Durchmesser der aktuellen Dicke (in Bildschirm-px) in der
  aktuellen Farbe. Im Radierer-Modus ein Kreis-Outline mit dem Radierer-Radius.
- **Palette** (Defaults, in der Config änderbar):
  1. `#1a1a1a` (schwarz)
  2. `#e03131` (rot)
  3. `#1971c2` (blau)
  4. `#2f9e44` (grün)
  5. `#f08c00` (orange)
  6. `#9c36b5` (lila)
- Default-Dicke: 1,5 pt.

### 9.3 Radierer (`ink/eraser.rs`)
- **Strich-Radierer**: Jeder Strich, dessen Polylinie (plus halbe Strichbreite) den Radierkreis
  berührt, wird komplett entfernt.
- Radius: 10 logische px, umgerechnet in pt beim aktuellen Zoom.
- Hit-Test: Abstand Punkt–Segment. Vorfilter über die Bounding-Box pro Strich.
- Alle während **eines** Ziehens entfernten Striche ergeben **ein** Undo-Schritt.
- Radiert werden nur mizu-eigene Striche, keine fremden Annotationen.
- **Radier-Ende des Stifts** (falls vorhanden): radiert im Draw-Modus immer, egal welches Werkzeug
  gerade aktiv ist. Das aktive Werkzeug ändert sich dadurch nicht.
- Hit-Test über ein grobes **Raster pro Seite** (Zellen à 32 pt, Zelle → Strich-IDs), damit auch
  bei Tausenden Strichen pro Seite nur wenige Striche geprüft werden.

### 9.4 Undo/Redo (`ink/history.rs`)
- `enum Edit { Add(Stroke), Remove(Vec<Stroke>) }`, Undo-Stack und Redo-Stack, unbegrenzt pro Sitzung.
- Eine neue Änderung leert den Redo-Stack.
- **Dirty-Flag**: Die Undo-Stack-Länge beim letzten Speichern wird gemerkt. Dirty heißt, die
  aktuelle Länge oder der Inhalt weicht davon ab (Implementierung über einen Zähler
  `saved_revision` vs. `revision`).
- `u` und `<C-r>` funktionieren in Normal **und** Draw. Nach Undo/Redo springt die Ansicht
  **nicht** (anders als Vim). Ist die betroffene Seite nicht sichtbar, kommt eine Meldung
  `Undo on page 12`.

### 9.5 Stift mit Druck und Pinch: `platform/`

Was `winit` nicht liefert, liefert ein kleines plattformspezifisches Modul. Alle Module speisen
dieselben Events in die App ein:
```rust
enum PlatformEvent {
    PinchBegin { pos }, PinchUpdate { scale_delta, pos }, PinchEnd,
    PenDown { pos, pressure, eraser: bool }, PenMove { pos, pressure },
    PenUp, PenProximityOut,
}
```
Grundregeln:
- **Immer zuerst prüfen**, ob die aktuelle `winit`-Version das Feature selbst kann. Wenn ja, wird
  es benutzt, und das Plattformmodul entfällt für diesen Fall.
- Fehlt das Protokoll bzw. die API zur Laufzeit (z. B. ein Compositor ohne tablet-v2), loggt mizu
  das einmal und fällt still auf Maus-Verhalten zurück. Es gibt nie einen Absturz.

Plattformen:
- **Wayland (Hauptziel)**:
  - Über `raw-window-handle` kommt man an das `wl_display` von winit. Darauf wird mit
    `wayland_client::Backend::from_foreign_display` eine eigene Verbindung angelegt, mit
    **eigener Event-Queue**.
  - Gebunden werden `wl_seat`, `zwp_pointer_gestures_v1` (Pinch, auf einem eigenen `wl_pointer`
    vom Seat) und `zwp_tablet_manager_v2` (`tablet_tool`: `pressure`, `motion`, `down`/`up`,
    Tool-Typ `pen`/`eraser`).
  - Die eigene Queue wird nicht blockierend in `about_to_wait` per `dispatch_pending` abgearbeitet.
    Winit liest denselben Socket, wird also bei neuen Events ohnehin geweckt.
  - Events werden nur für unsere `wl_surface` ausgewertet (die Surface-ID stammt aus dem
    Window-Handle).
  - Wichtig: Sobald tablet-v2 gebunden ist, schickt der Compositor Stift-Events **nur noch** über
    tablet-v2 und nicht mehr als Maus-Events. Der Stift muss deshalb auch in Normal-Modus und
    Kommandozeile wie eine linke Maustaste funktionieren (Verschieben, Links klicken).
  - Der Cursor über dem Fenster bei Stift-Nähe wird über `zwp_tablet_tool_v2.set_cursor` gesetzt.
- **Windows**: Das Fenster wird gesubclasst (`SetWindowSubclass`), um `WM_POINTERDOWN`,
  `WM_POINTERUPDATE` und `WM_POINTERUP` abzufangen. Bei `PT_PEN` liefert `GetPointerPenInfo`
  `pressure` (0–1024) und `PEN_FLAG_ERASER` bzw. `PEN_FLAG_INVERTED`. Danach wird an winit
  weitergereicht.
- **macOS**: `NSEvent.addLocalMonitorForEventsMatchingMask` (Mouse-Down/Dragged/Up, Tablet-Point,
  Tablet-Proximity) liest `pressure` und `pointingDeviceType == NSPointingDeviceTypeEraser`.
  Die Events werden unverändert weitergegeben.
- **X11** (Fallback, niedrigste Priorität innerhalb von M8): XInput 2.4 über `x11rb`
  - Gesten über `XI_GesturePinchBegin`/`Update`/`End`
  - Stift-Druck über das Valuator-Axis-Label `Abs Pressure` auf dem Tablet-Device

Verarbeitung:
- **Druckkurve**: `breite_i = width · (0.25 + 0.75 · p_i^0.75)`, mit `p` geglättet
  (exponentieller Mittelwert, α = 0.4). Hat ein Strich keinen Druck, gilt eine konstante Breite.
- Darstellung variabler Breite: `lyon` `StrokeOptions` mit `variable_line_width`
  (Breite als Vertex-Attribut).
- Pinch: Der Zoom wird multiplikativ um den Gesten-Mittelpunkt angewendet, wie bei
  `Ctrl`+Rad. Kacheln werden erst nach dem Ende der Geste plus 120 ms nachgerendert.

---

## 10. Speichern und Laden der Striche (`doc/annots.rs`)

### 10.1 Format im PDF
Jeder Strich wird eine Standard-**Ink-Annotation**. Damit erscheint er auch in Okular, Zathura,
Acrobat und Browsern.
- `/Subtype /Ink`
- `/InkList [[x1 y1 x2 y2 …]]`
- `/C [r g b]` (0..1)
- `/BS << /W width >>`
- `/CA 1`
- `/NM (mizu-<uuid>)`: **daran erkennt mizu eigene Striche**
- `/T (mizu)`
- Appearance-Stream von MuPDF generieren lassen (`pdf_update_annot`), damit andere Viewer ihn
  korrekt anzeigen.
- **Striche mit Druck**:
  - `/BS /W` enthält die mittlere Breite.
  - Zusätzlich gibt es einen eigenen Schlüssel `/MizuP [p1 p2 …]` mit den Druckwerten.
    Beim Laden hat er Vorrang.
  - Der Appearance-Stream (`/AP /N`) wird **selbst** geschrieben: das Strich-Outline variabler
    Breite als gefülltes Polygon in Seitenkoordinaten. So sehen fremde Viewer die echte
    Druckform. Danach darf kein `pdf_update_annot` das AP überschreiben (MuPDF-API dafür
    prüfen, ggf. `/AP` nach dem Update setzen).
- MuPDFs Annotations-Funktionen erwarten Koordinaten im fitz-Seitenraum und rechnen selbst in den
  PDF-Raum um (inklusive `/Rotate` und CropBox). Das wird mit der rotierten Fixture-Seite
  **getestet**.
- Falls `mupdf-rs` die nötigen Funktionen nicht anbietet (`pdf_set_annot_ink_list`,
  `pdf_set_annot_color`, `pdf_set_annot_border_width`, `/NM` setzen bzw. lesen,
  `pdf_delete_annot`, Ink-List lesen), werden sie in `doc/ffi_ext.rs` über `mupdf-sys` ergänzt.
  Dort liegt ein dünner, sicherer Wrapper; alles `unsafe` bleibt in dieser Datei.

### 10.2 Laden
1. Beim Öffnen liest der Hauptthread (über einen dedizierten Lade-Job) alle Ink-Annotationen mit
   `/NM` beginnend mit `mizu-` und erzeugt daraus `Stroke`s.
2. **Jeder Render-Worker löscht diese Annotationen in seiner eigenen In-Memory-Kopie**, bevor er
   eine Seite rendert (einmal pro Seite). mizu-Striche zeichnet also nur die GPU-Ebene; sie
   erscheinen nie doppelt und bleiben editierbar.
3. Fremde Annotationen (von anderen Programmen) rendert MuPDF ganz normal. mizu kann sie nicht bearbeiten.

### 10.3 Speichern (`:w`)
1. Läuft auf einem eigenen Thread. Die UI blockiert nicht; die Statuszeile zeigt `Saving…`.
2. Die Datei wird frisch von der Festplatte geöffnet, alle `mizu-`-Annotationen werden gelöscht
   und alle aktuellen Striche als neue Ink-Annotationen angelegt.
3. Gespeichert wird in eine temporäre Datei **im selben Verzeichnis** (`tempfile::NamedTempFile::new_in`),
   als vollständiger Save ohne Garbage-Collection-Optionen, die die Struktur stark umbauen.
   Danach `fsync` und `persist` (atomisches Rename) über das Original. Dateirechte des Originals
   werden übernommen.
4. Bei Erfolg: `saved_revision = revision` und Meldung `"skript.pdf" written`. Bei Fehler (z. B.
   verschlüsseltes PDF, keine Schreibrechte) erscheint die Meldung in der Statuszeile, und das
   Dirty-Flag bleibt.
5. Das Auto-Reload (11.3) ignoriert das Dateiereignis des eigenen Speicherns. Dafür wird ein
   Flag bzw. Zeitstempel gesetzt und die mtime nach dem Rename verglichen.

---

## 11. Weitere Viewer-Funktionen

### 11.1 Suche
- `/text<CR>` sucht ab der aktuellen Seite vorwärts (`?` rückwärts) mit `Page::search`, seitenweise
  auf einem Worker. Ergebnisse kommen inkrementell.
- Groß- und Kleinschreibung: Smartcase wie in Vim (enthält die Suche Großbuchstaben, ist sie
  case-sensitiv). Falls MuPDF nur case-insensitiv sucht, wird für den case-sensitiven Fall
  nachgefiltert.
- Treffer werden als halbtransparente Rechtecke (Quads) gezeichnet. Der aktuelle Treffer ist
  kräftiger hervorgehoben.
- `n` / `N` springen zum Treffer und zentrieren ihn vertikal. Die Statuszeile zeigt `[3/17]`.
- Kein Treffer: `E486: Pattern not found: text`.

### 11.2 Outline (Inhaltsverzeichnis), wie sioyek
- `o` öffnet ein Overlay mit einer Liste, eingerückt nach Ebene, plus Seitenzahl.
- `j` / `k` (bzw. Pfeiltasten) wählen aus, `<CR>` springt (Eintrag in die Sprungliste),
  `<Esc>` schließt. Tippen filtert die Liste (case-insensitive Substring).
- Gibt es keine Outline: Meldung `No outline`.

### 11.3 Auto-Reload
- `notify` beobachtet das **Verzeichnis** der Datei, weil Editoren und LaTeX oft per Rename
  ersetzen. Ereignisse werden auf 200 ms entprellt.
- Beim Reload werden Seite, Offset und Zoom beibehalten, alle Caches verworfen und die Striche
  neu geladen.
- Gibt es **ungespeicherte Striche**, wird nicht automatisch neu geladen. Stattdessen erscheint
  `File changed on disk. :e! to reload (discards drawings)`.

### 11.4 Links
Klick im Normal-Modus:
- interne Links springen (Sprungliste)
- externe Links öffnen über `open::that`

Fährt man über einen Link, wird der Cursor zur Hand.

### 11.5 Session (`session.rs`)
- Datei: `<state_dir>/mizu/session.json`, auf Linux `~/.local/state/mizu/`.
- Pro kanonischem Dateipfad: Seite + Offset, Zoom-Modus/Zoom, Dunkelmodus, Marken.
- Wird beim Schließen und beim Wechsel der Datei geschrieben, atomisch.
- Maximal 500 Einträge (LRU).

### 11.6 Passwortgeschützte PDFs
Braucht das Dokument ein Passwort, fragt die Kommandozeile `Password: `. Die Eingabe wird als `*`
angezeigt. Drei Versuche, danach `Wrong password` und ein leeres Fenster.

---

## 12. Statuszeile und Oberfläche

- Eine Zeile am unteren Rand. Höhe = Schrifthöhe + 6 px. Die Schrift ist eingebettet
  (`include_bytes!`) und **nicht** von Systemschriften abhängig, weil diese unter NixOS
  unzuverlässig gefunden werden.
- Links:
  - der Modus (`NORMAL` / `DRAW` / `ERASE`)
  - der Dateiname
  - `[+]`, wenn dirty
- Rechts:
  - im Draw-Modus ein Farbpunkt und die Dicke (`● 1.5pt`)
  - Suchstatus
  - `12/148`
  - `125%`
- Meldungen und Fehler ersetzen den linken Teil, bis zur nächsten Taste bzw. 4 s lang.
  Fehler werden rot dargestellt.
- Im Command- und Search-Modus wird die Zeile zur Eingabezeile (`:` bzw. `/` plus Text und Cursor).
- Farben: Im Dunkelmodus schwarzer Hintergrund und weiße Schrift. Im hellen Modus weißer
  Hintergrund und schwarze Schrift.
- Text wird nur neu geshapt (glyphon), wenn sich der Inhalt ändert, nicht jeden Frame.
- Mit `statusbar = false` in der Config ist die Statuszeile ausgeblendet. Sie erscheint dann nur
  bei Eingaben und Meldungen.
- Ohne Datei (Start ohne Argument): leeres Fenster, zentriert dezent `mizu — :e <file>`.
- Fenstertitel: `skript.pdf — mizu`.
- **Wayland**: Die `app_id` wird über
  `WindowAttributesExtWayland::with_name("io.github.notflask.Mizu", "mizu")` gesetzt, damit
  Niri und Hyprland Fensterregeln und Icon zuordnen können.

---

## 13. Konfiguration

Pfad: `directories::ProjectDirs` → Linux `~/.config/mizu/config.toml`, macOS
`~/Library/Application Support/io.github.notflask.Mizu/config.toml`, Windows
`%APPDATA%\notflask\Mizu\config\config.toml`.

- Fehlt die Datei, gelten die Defaults.
- Ist die Datei ungültig, gelten ebenfalls die Defaults, und die Statuszeile zeigt den Parse-Fehler.
- `README.md` dokumentiert ein vollständiges Beispiel:

```toml
dark_by_default = false
statusbar = true
scroll_step = 60          # logische px
zoom_step = 1.2
tile_cache_mb = 256

[dark]
background = "#000000"
foreground = "#ffffff"
# separator = "#1a1a1a"   # Linie zwischen Seiten; Standard: keine

[pen]
width = 1.5
palette = ["#1a1a1a", "#e03131", "#1971c2", "#2f9e44", "#f08c00", "#9c36b5"]

[keys.normal]
"<C-n>" = "toggle_dark"   # eigene Belegungen überschreiben/ergänzen Defaults
"D" = "none"              # Default entfernen

[keys.draw]
"x" = "toggle_eraser"
```

Action-Namen sind die `snake_case`-Varianten von `enum Action`. Sie stehen in `README.md` als
Tabelle (generiert oder von Hand, Hauptsache vollständig).

---

## 14. CLI

```
mizu [FILE] [--page N]
mizu --version | --help
```
`--page` überschreibt die gespeicherte Session-Position. Exit-Code 0, außer beim Fehler
„Datei nicht lesbar“: dann gibt mizu eine Meldung auf stderr aus und beendet sich mit Code 1
(nur wenn die Datei per CLI übergeben wurde).

---

## 15. Icons

### 15.1 Motiv
Ein Blatt Papier im Hochformat mit abgerundeten Ecken und **Eselsohr** oben rechts. Eine sanfte
**Welle** teilt das Blatt bei etwa 58 % Höhe:
- **oben** papierweiß (`#F6F5F2`) mit 3 dunkelgrauen „Textzeilen“ (abgerundete Balken)
- **unten** tiefes Wasserblau, als Verlauf `#22324F` → `#1A2438`, mit 2 hellen Textzeilen
  (`#C9D6EA`), die die oberen fortsetzen, aber invertiert

Das Motiv erzählt also „PDF + Wasser + Dunkelmodus“. Die Welle hat eine feine hellere Kante
(`#4C7BC0`, 2 % Breite) als Wasserlinie. Stil: flach, ruhig, keine Spiegelungen außer wo die
Plattform es verlangt (macOS).

### 15.2 Quellen (`assets/icons/src/`, von Hand gepflegte SVGs, Raster 1024×1024)
| Datei | Zweck |
|---|---|
| `mizu-macos.svg` | macOS: Blatt auf **Squircle**-Hintergrund nach Apples Raster (824×824 px Squircle zentriert im 1024er Canvas, Superellipse mit kontinuierlicher Krümmung, Radius ≈ 185 px). Hintergrund: sehr heller Blau-Grau-Verlauf `#EEF2F7` → `#D9E1EC`. Blatt mit weichem Schatten (y +12 px, Blur 24 px, 20 % Schwarz). Oben ein dezenter Lichtverlauf |
| `mizu.svg` | Linux und Windows (groß): **nur das Blatt**, ohne Hintergrundplatte, leichter Schatten, füllt ca. 80 % Höhe |
| `mizu-small.svg` | Windows/Linux 16–32 px: vereinfacht. Keine Textzeilen, Welle dicker, Kanten auf das Pixelraster gelegt (auf 32er Raster gezeichnet) |
| `mizu-symbolic.svg` | Linux symbolisch, 16×16, einfarbig `#2e3436`: Blatt-Umriss + Welle als Linie |

### 15.3 Generierung (`cargo xtask icons`)
- `resvg` rendert die PNGs: 16, 24, 32, 48, 64, 128, 256, 512, 1024.
  Für ≤ 32 px wird `mizu-small.svg` benutzt.
- **Linux**:
  - `generated/linux/hicolor/scalable/apps/io.github.notflask.Mizu.svg` (Kopie von `mizu.svg`)
  - `…/symbolic/apps/io.github.notflask.Mizu-symbolic.svg`
  - `…/<N>x<N>/apps/io.github.notflask.Mizu.png`
- **Windows**: `generated/windows/mizu.ico` mit 16, 20, 24, 32, 40, 48, 64, 256 (256 als PNG im ICO).
- **macOS**: `generated/macos/mizu.icns` mit allen Größen inkl. @2x (16 bis 1024).
- Die erzeugten Dateien werden **committet**, damit Packaging das xtask nicht braucht.
- Außerdem wird `generated/preview.png` erzeugt: alle Varianten nebeneinander, einmal auf hellem
  und einmal auf dunklem Grund. Das dient zur Sichtkontrolle.

### 15.4 Einbindung
- Windows: `build.rs` bettet `mizu.ico` in die `.exe` ein (nur bei `target_os = "windows"`).
  Zusätzlich gilt `#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]`.
- X11 und Windows: Zur Laufzeit wird das Fenster-Icon über `Window::set_window_icon` mit dem
  256er PNG gesetzt.
- Wayland: Das Icon kommt über `app_id` und die `.desktop`-Datei.
- macOS: `.icns` im `.app`-Bundle.

---

## 16. Packaging und Plattformen

### 16.1 NixOS (Hauptplattform) – `flake.nix`
- `devShells.default`:
  - Rust-Toolchain (aus nixpkgs oder `rust-overlay`), `rust-analyzer`, `clippy`, `rustfmt`
  - `pkg-config`, `clang` / `libclang` (für bindgen in `mupdf-sys`, `LIBCLANG_PATH` setzen)
  - `gnumake`, `python3` (falls der MuPDF-Build sie braucht)
  - Laufzeit-Bibliotheken, die per `dlopen` geladen werden, über `LD_LIBRARY_PATH`:
    `vulkan-loader`, `wayland`, `libxkbcommon`, `libGL`, `xorg.libX11`, `xorg.libXcursor`,
    `xorg.libXi`, `xorg.libXrandr`
- `packages.default`:
  - `rustPlatform.buildRustPackage` mit `cargoLock.lockFile = ./Cargo.lock`
  - `nativeBuildInputs = [ pkg-config rustPlatform.bindgenHook makeWrapper ]`
  - `postInstall`: Desktop-Datei und hicolor-Icons installieren
  - `postFixup`: `wrapProgram $out/bin/mizu --prefix LD_LIBRARY_PATH : <die Laufzeit-Libs>`
- `apps.default` → `nix run github:notflask/mizu file.pdf`
- **Prüfen**, ob der MuPDF-Build von `mupdf-sys` in der Nix-Sandbox ohne Netz läuft (die Quellen
  sind im Crate vendored). Falls er Probleme macht: Abhängigkeiten nachziehen, nicht auf
  System-MuPDF ausweichen, außer es geht nicht anders. Dann im Plan bzw. README dokumentieren.
- Desktop-Datei:
  - `Name=mizu`
  - `Exec=mizu %f`
  - `Icon=io.github.notflask.Mizu`
  - `MimeType=application/pdf;`
  - `Categories=Office;Viewer;`
  - `StartupWMClass=io.github.notflask.Mizu`

### 16.2 Andere Linux-Distributionen
- Ein Release-Tarball mit Binary, `.desktop`-Datei und Icons.
- Ein **AppImage**, gebaut mit `linuxdeploy` bzw. `appimagetool` in `release.yml`.
  - Basis ist ein älteres Ubuntu-LTS, damit die glibc-Kompatibilität passt.
  - `libvulkan`, `libwayland-client` und `libxkbcommon` werden vom Host benutzt und
    **nicht** gebündelt.

### 16.3 Windows
- Ein `.zip` mit `mizu.exe`; MuPDF ist statisch gelinkt, es gibt also keine DLLs.
- Zusätzlich ein **MSI-Installer** über `cargo-wix` (WiX). Er enthält:
  - einen Startmenü-Eintrag
  - die Registrierung als „Öffnen mit“-Programm für `.pdf` (über `RegisteredApplications` und
    `Capabilities`). Die Standard-App wird **nicht** ungefragt umgestellt.
  - eine Deinstallation, die alles wieder sauber entfernt

### 16.4 macOS
- `.app`-Bundle: `Info.plist` mit
  - `CFBundleIdentifier=io.github.notflask.Mizu`
  - `CFBundleDocumentTypes` für `com.adobe.pdf` (Rolle Viewer)
  - `CFBundleIconFile=mizu`
- Ad-hoc signiert (`codesign -s -`), ausgeliefert als `.dmg` (`hdiutil`).
- **Notarisierung**: `release.yml` signiert und notarisiert automatisch (`codesign` mit
  Developer-ID, `xcrun notarytool submit --wait`, `xcrun stapler staple`), **wenn** die Secrets
  `APPLE_CERT_P12`, `APPLE_CERT_PASSWORD`, `APPLE_ID`, `APPLE_TEAM_ID` und
  `APPLE_APP_PASSWORD` gesetzt sind. Dafür braucht man einen kostenpflichtigen
  Apple-Developer-Account. Ohne Secrets wird ad-hoc signiert, und die README erklärt den
  Rechtsklick → Öffnen beim ersten Start.
- Universal Binary (`aarch64` + `x86_64`, per `lipo`).
- Datei öffnen per Finder: Das `Opened`-Event für Dateien bzw. URLs aus `winit` (macOS)
  verarbeiten.

### 16.5 CI (GitHub Actions)
- `ci.yml`, bei Push und PR:
  - Matrix `ubuntu-latest`, `windows-latest`, `macos-latest` mit `cargo fmt --check`,
    `clippy -D warnings`, `cargo test`, `cargo build --release`
  - zusätzlich ein Job `nix build` auf Ubuntu (Nix-Installer-Action)
  - Cache: `Swatinem/rust-cache`
- `release.yml`, bei Tag `v*`: Artefakte bauen und an einen GitHub-Release hängen:
  - Linux-Tarball
  - AppImage
  - Windows-ZIP
  - MSI
  - macOS-DMG
- CI prüft außerdem die Binary-Größe (Abschnitt 19.5): Der Job schlägt fehl, wenn das Limit
  überschritten wird.

---

## 17. Meilensteine

Jeder Meilenstein endet mit einem lauffähigen Programm und grünen Checks.

| # | Inhalt | Fertig, wenn … |
|---|---|---|
| **M0** | Cargo-Projekt, `flake.nix`-devShell, `LICENSE`, CI-Grundgerüst, Fenster mit wgpu, eine Seite per MuPDF als **eine** Textur (noch ohne Kacheln) anzeigen | `nix develop -c cargo run -- tests/fixtures/sample.pdf` zeigt Seite 1 unter Wayland |
| **M1** | Layout (alle Seiten), Kamera, Scrollen mit Mausrad/Touchpad, Ziehen, Zoom zum Cursor, FitWidth/FitPage, Worker-Pool, **Kacheln**, Vorschau-Ebene, LRU-Cache, Entprellung, HiDPI/fraktionale Skalierung | Ein 300-Seiten-Skript scrollt flüssig. Bei 800 % Zoom ist der Text scharf, und während des Zoomens wird nie ein leeres Bild angezeigt. Unit-Tests für Kamera und Layout |
| **M2** | Statuszeile (glyphon + eingebettete Schrift), Keymap-Engine mit Counts und Sequenzen, alle Normal-Bindings außer Suche, Outline und Draw, Kommandozeile mit `:q`, `:<N>`, `:e` (+ Tab-Completion), Config-Datei, Session (letzte Position, Marken, Sprungliste), Drag & Drop, `app_id` | Alle Tabellen in 8.3 und 8.6 (soweit schon sinnvoll) funktionieren. Unit-Tests für Key-Parser und Keymap |
| **M3** | Dunkelmodus: Recolor-Shader, CPU-Referenz + Tests, `D`, `:dark`/`:light`, Config-Farben, Session | Umschalten erfolgt ohne Neurendern sofort, und Farben behalten ihren Farbton |
| **M4** | Draw-Modus: Stift, Live-Darstellung (lyon, MSAA), Glättung + RDP, Palette, Dicke, Cursor, Radierer (inkl. Rechtsklick), Undo/Redo, Dirty-Flag, `[+]`, E37 bei `:q` und beim Schließen | Man kann flüssig mit Maus und Stift schreiben und radieren; Undo/Redo stimmt. Tests für Eraser-Hit-Test, History und RDP |
| **M5** | Speichern/Laden: `doc/annots.rs`, `ffi_ext.rs`, `:w`, `:w <pfad>`, `:wq`, `:x`, `ZZ`, `ZQ`, atomisches Speichern, Worker entfernen mizu-Annots vor dem Rendern | Round-Trip-Test: Striche anlegen, speichern, neu laden, sind identisch (inkl. rotierter Seite). Manuell: Die Striche erscheinen korrekt in Okular/Zathura/Firefox |
| **M6** | Suche (`/`, `?`, `n`, `N`, Smartcase), Outline-Overlay, Links, Auto-Reload (inkl. Schutz bei Dirty und eigenem Save), Passwortabfrage | Funktioniert mit einem LaTeX-Workflow (`latexmk -pvc`): Die Ansicht bleibt beim Neuladen an derselben Stelle |
| **M7** | Icons (SVG-Quellen, xtask, generierte Dateien, preview.png), Einbindung pro Plattform, Desktop-Datei, Nix-Paket (`nix build`, `nix run`), Windows-ZIP + MSI, AppImage, macOS-`.app`/`.dmg` (Universal, Notarisierung bei vorhandenen Secrets), `release.yml`, README fertig | `nix run .` startet mizu mit Icon in Niri/Hyprland-Launchern. Die CI baut alle drei Plattformen und alle Pakete |
| **M8** | `platform/`-Module (9.5): Pinch-Zoom auf Wayland/X11, Stift mit Druck und Radier-Ende auf Wayland, Windows, macOS und X11. Variable Strichbreite (lyon), `/MizuP` und eigener Appearance-Stream beim Speichern | Unter Niri und Hyprland: Pinch zoomt flüssig, Stiftdruck verändert sichtbar die Breite, das Radier-Ende radiert. Die gespeicherte Datei zeigt die Druckform auch in Okular/Firefox. Ohne Tablet bzw. Protokoll fällt mizu sauber auf die Maus zurück |
| **M9** | **Optimierungs-Durchgang**: Profiling (Tracy) aller Hauptpfade, alle Ziele aus Abschnitt 19 messen und erreichen, Benchmarks vervollständigen, Ergebnisse in `docs/TESTING.md` | Alle Messwerte aus 19.1 sind erreicht und dokumentiert |

---

## 18. Tests

- **Unit-Tests**:
  - Kamera (Zoom zum Cursor ist invariant, Clamping)
  - Layout
  - Key-Parser und Keymap (Counts, Sequenzen, Overrides aus TOML)
  - Kommando-Parser
  - Recolor-CPU-Referenz
  - Glättung und RDP
  - Eraser-Hit-Test
  - History und Dirty-Flag
  - Session-Serialisierung
- **Integrationstests** (`tests/`, mit MuPDF, ohne Fenster):
  - Annotations-Round-Trip
  - rotierte Seite
  - gemischte Seitengrößen
  - fremde Annotationen bleiben beim Speichern erhalten
  - atomisches Speichern lässt bei einem Fehler das Original unverändert
- **Fixtures** werden per kleinem Testhelfer mit MuPDF erzeugt oder als winzige PDFs (< 50 KB)
  committet.
- **Manuelle Checkliste** pro Meilenstein in `docs/TESTING.md` pflegen (Wayland/Niri, Hyprland,
  HiDPI, X11-Fallback, Windows, macOS, Stift mit Druck und Radier-Ende, Touchpad-Pinch) inklusive
  der Performance-Messwerte aus 19.1.

## 19. Performance und Optimierung (höchste Priorität)

mizu muss **sehr gut optimiert** sein. Die folgenden Regeln gelten von M0 an, nicht erst in M9.
M9 ist nur der abschließende Prüf- und Feinschliff-Durchgang.

### 19.1 Messbare Ziele
Referenz: ein Mittelklasse-Laptop mit integrierter GPU und ein 300-seitiges Vorlesungsskript mit
Bildern (≤ 20 MB).

| Metrik | Ziel |
|---|---|
| Start bis Fenster sichtbar | < 100 ms |
| Start bis erste scharfe Seite | < 250 ms |
| CPU-Zeit pro Frame (Hauptthread) beim Scrollen/Zoomen/Zeichnen | < 2 ms (also auch 144-Hz-tauglich) |
| Frame-Drops beim Scrollen und Zoomen | keine, bei 60/120/144 Hz |
| Latenz Eingabe → sichtbares Strichsegment | nächster Frame |
| Zeit bis scharfe Kacheln nach Zoom-Ende | < 150 ms (+120 ms Entprellung) für einen Bildschirm |
| CPU im Leerlauf | **0 %** (keine Timer, kein Polling, keine Redraws ohne Grund) |
| RAM (RSS) ohne Kachel-Cache | < 120 MB |
| GPU-Speicher | innerhalb der Budgets (Kacheln 256 MB, Vorschauen 48 MB) |
| Speichern von 1000 Strichen | < 300 ms, UI bleibt flüssig |
| Release-Binary (Linux, gestrippt) | < 30 MB |

### 19.2 Build
```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
strip = true
debug = false

[profile.profiling]          # für Tracy und perf
inherits = "release"
debug = true
strip = false
```
- MuPDF wird mit Optimierung gebaut. Prüfen, welche Flags `mupdf-sys` setzt.
- Ungenutzte MuPDF-Teile werden abgeschaltet, soweit das Crate es erlaubt (JavaScript,
  XPS/EPUB/HTML/SVG-Dokumenttypen usw.). Eingebettete Fallback-Fonts (z. B. das große CJK-Paket)
  fliegen nur raus, wenn PDFs mit nicht eingebetteten Fonts weiter lesbar bleiben. Die
  Entscheidung kommt mit Größenangabe in die README.
- `wgpu` ohne Default-Features. Aktiviert werden nur die nötigen Backends pro Plattform: Vulkan
  und GL auf Linux, DX12 auf Windows, Metal auf macOS, dazu WGSL.
- Alle anderen Crates werden mit minimalen Features eingebunden. Jede neue Abhängigkeit muss
  sich lohnen (Binary-Größe und Compile-Zeit prüfen, z. B. mit `cargo bloat`).

### 19.3 Hauptthread und Rendering
- Der Hauptthread macht **nur**: Input, Kamera, Draw-Calls bauen und Uploads. Rendern, Suchen,
  Laden, Speichern und Datei-I/O laufen alle auf Workern.
- **Keine Heap-Allokation pro Frame**: Vecs und Buffer werden wiederverwendet. Instanz- und
  Vertex-Buffer wachsen bei Bedarf (Verdopplung) und schrumpfen nie im laufenden Betrieb.
- Events werden zusammengefasst: Mehrere `CursorMoved`-/Scroll-Events zwischen zwei Frames
  werden alle verarbeitet, gezeichnet wird aber **einmal** (`request_redraw` ist idempotent).
- Kacheln: Texture-Arrays plus instanzierte Draw-Calls (6.2), Upload-Budget pro Frame,
  Culling per Bounding-Box.
- Striche:
  - **Ein** Vertex-Buffer pro Seite für alle fertigen Striche. Er wird nur neu gebaut, wenn sich
    die Striche dieser Seite ändern.
  - Der aktuelle Strich hat einen eigenen dynamischen Buffer. Beim Zeichnen werden nur neue
    Segmente tesselliert und angehängt; der Strich wird nicht jedes Mal komplett neu berechnet.
- Pipelines, Sampler und Bind-Group-Layouts werden einmal beim Start erstellt. Wo es unterstützt
  wird, kommt ein `wgpu::PipelineCache` zum Einsatz, gespeichert im Cache-Verzeichnis.
- Startup-Parallelität: Fenster und wgpu-Init laufen parallel zum Öffnen des PDFs und dem
  Rendern der ersten Seite. Das Fenster ist sofort da, mit Hintergrundfarbe aus der Session
  (schwarz im Dunkelmodus, damit nichts weiß aufblitzt).
- Seitengrößen beim Öffnen:
  - Erst messen. Dauert das Lesen aller Seitengrößen bei 1000 Seiten länger als 30 ms, werden
    die Größen der ersten sichtbaren Seiten sofort gelesen und der Rest im Hintergrund.
  - Bis dahin gilt die Größe der ersten Seite als Platzhalter. Kommt die echte Größe, wird das
    Layout korrigiert, wobei die aktuelle Seite fest verankert bleibt (kein Springen).

### 19.4 Worker
- Display-List-Cache pro Worker (6.1). Kacheln rendern aus der Display-List, nicht jedes Mal
  aus der Seite.
- Pixmap-Pool pro Worker, Rendern direkt in RGBA (6.2).
- Veraltete Jobs werden verworfen, bevor gerendert wird (Generation). Laufende Suchen sind
  abbrechbar, wenn eine neue Suche beginnt.
- Für Suche und Outline wird höchstens ein Worker gleichzeitig verwendet, damit das
  Kachel-Rendern nicht verhungert.
- MuPDF: Ob ICC-Farbmanagement abgeschaltet wird, entscheidet die Messung. Nur wenn es messbar
  schneller ist und Text und Bilder sichtbar gleich bleiben.

### 19.5 Speicher und Größe
- Budgets für Kacheln und Vorschauen (6.2) werden strikt per LRU eingehalten.
- Der Display-List-Cache ist auf ~8 Seiten pro Worker begrenzt. Text-Seiten für die Suche werden
  nicht dauerhaft gecacht.
- Bei sehr vielen Strichen: Daten kompakt halten (`f32`, keine Strukturen pro Punkt).
- CI prüft die Binary-Größe (Ziel aus 19.1).

### 19.6 Messen statt raten
- `mizu --stats` blendet in der Statuszeile ein:
  - Frame-Zeit (avg/max über 1 s)
  - Kachel-Latenz
  - Cache-Belegung
  - Anzahl der Draw-Calls
- Cargo-Feature `profile`: Mit `profiling`-Makros und Tracy-Backend sind alle Hauptpfade
  instrumentiert. Ohne das Feature kostet das nichts.
- `criterion`-Benchmarks in `benches/`:
  - Kachel rendern (aus der Display-List)
  - Strich tessellieren (100 / 1000 Punkte)
  - Radierer-Hit-Test (10 000 Striche)
  - Keymap-Lookup
  - Recolor-CPU-Referenz
  - Annotationen speichern (1000 Striche)
- Am Ende jedes Meilensteins werden die relevanten Werte aus 19.1 gemessen und in
  `docs/TESTING.md` eingetragen, mit Datum und Commit. Verschlechtert sich ein Wert gegenüber
  dem vorherigen Eintrag, ist das ein Bug und wird vor dem Weitermachen behoben.
