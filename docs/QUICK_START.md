# Start drawing in Dudel

Dudel is a drawing application for pen tablets on Windows. Save editable work as
a `.dudel` project and export PNG copies when you want to share the image.

When Dudel starts, the welcome window offers a new canvas, **Open…** and your
recent projects, with the keys worth learning first. Untick *Show this window
when Dudel starts* to skip it; **Help > Welcome screen** brings it back. The
**Help** menu also links to this guide and to the problem report form.

The Windows x64 build links the C runtime into the application; a separate Visual
C++ redistributable is not required.

## Find actions and keep track of your target

In current development builds, **Edit > Find a command** (`Ctrl+K` or
`Ctrl+Shift+P`) searches common file, layer, selection, view and tool actions.
Type an action or a familiar term such as "mirror" or "transparent PNG".
Use Up/Down and Enter, or click a result. Unavailable actions show a reason;
Escape closes search without clearing the canvas selection. Shortcuts appear
beside results and follow your saved bindings.

### Customize shortcuts

Open **View > Settings > Shortcuts**, or **Help > Keyboard shortcuts > Customize
shortcuts**. Search for a command, click its current binding (or **Unassigned**),
press the new keys and click **Apply shortcut**. This replaces that command's
bindings. Conflicts name the command that already uses those keys; **Unbind** it
first if you want to move its shortcut. Escape cancels assignment.

**Default** restores one command; **Restore defaults** restores all commands
after confirmation. **Export profile** saves your bindings as a `.dudelkeys`
file; **Import profile** replaces them after checking the complete file. Invalid
or conflicting profiles leave your current bindings intact. Bindings are saved
in application settings, independently of artwork files. Menus, tooltips,
command search and the shortcut reference display the active bindings.

Tool bindings retain tap-to-select and hold-to-switch behavior. Clipboard keys,
Tab project/control navigation, Enter/Escape, arrow keys, Delete/Backspace,
F6/F11, Space, R rotation, F resizing and Alt gestures retain their standard
roles. Plain Tab can be removed from the panel-toggle command. Shortcuts in
the rest of this guide describe the defaults. Common built-in commands can be
rebound; extension commands and pointer gestures are outside this editor.

The strip above the canvas names the active artwork, vector, mask or reference
target. It offers explicit actions for locked layers, hidden layers and their
folders, transparency locks, and tools that need a raster layer. **Switch to
artwork** leaves mask/reference editing. **Deselect** removes an active selection.
These actions use the normal editing and undo paths; selecting a vector layer
never automatically rasterizes it.

The Layers panel searches names and filters by raster, vector, adjustment,
folder or reference type. Search ignores case and finds children inside collapsed
folders without expanding them or changing your selection. The **Types** menu
(named for the selected type when filtered) includes **Clear filters** and
**Compact rows**, which uses smaller rows with details in their tooltips.
**Reveal** clears filters, expands the active entry's parent folders and scrolls
to it. Clear filters before dragging layers into a different stack position.
In very short windows, click **LAYERS** to open these navigation controls.

### Select and edit several layers

**Ctrl-click** layer names or artwork thumbnails to add/remove individual
layers. **Shift-click** selects a range of currently displayed artwork rows;
Ctrl+Shift-click adds that range. Collapsed children are excluded unless a
search reveals them. The brighter row remains the active painting target.
Plain click or **Only active** returns to a single layer.

With several layers selected, **Batch** above the rows offers Show, Hide, Lock,
Unlock, Move up/down, Group and Delete. Move changes stack order, not pixel
positions. Move and Group require a shared parent folder and preserve relative
order. Nonadjacent layers group at the highest selected position. Each successful
batch is one undo step; rejected or unchanged operations create none.

A selected folder covers its descendants, even if they are selected separately.
Visibility and lock actions change the selected folder's own flag, preserving
its children's individual flags. Structural actions reject locked targets,
locked contents or locked parents. Deleting the last artwork layer is rejected.
Earlier queued drawing settles before a batch is applied.

Filtering preserves the selection; the count reports selected rows hidden by
filters or collapsed folders. **Batch acts on the whole selection**, including
those hidden rows. Multi-layer dragging is disabled; use Move up/down or Group.
References retain their separate selection controls. Painting, masks, transforms,
merging and duplication still target the active layer. Selection is session UI
state, not part of the saved artwork or undo history.

## Create and draw

1. Choose **File > New canvas** (`Ctrl+N`). Choose **Sketch**, **Square**, or **4K**,
   or enter custom width and height. **Swap W/H** changes orientation. The form
   shows pixel count and memory per fully painted layer. Then choose
   **Create canvas**. Each side can be up to 16,384 pixels and the canvas up to 100 megapixels
   (10,000 x 10,000, or a tall 800 x 16,384 strip). Very large canvases with many fully painted
   layers can reach the 2 GiB artwork budget; Dudel reports that clearly. The new canvas
   opens in its own tab (see [Tabs](#tabs-preview-window-and-folders)); at the tab
   limit it replaces the current project after Save, Don't save or Cancel.
2. The tool rail has four paint tools: **Pencil**, **Pen**, **Brush** and
   **Airbrush**. Press **B** for the paint tools and tap **B** again to step
   through them. Each one lists its own presets in the brush selector and keeps
   its own size and settings, so switching back finds the brush as you left it.
   Adjust Size and Opacity in the options bar; click the foreground color chip to
   choose a color. **E** selects Eraser; **X** swaps foreground/background colors.
   The cursor shows the brush tip's real shape: the outline of round and square
   tips, and the tip image itself (in the current color) for textured tips. While
   you draw it follows the ink, growing, shrinking and turning with pressure,
   stroke direction or tilt; the eraser's outline is dashed.
   The Eraser either erases with the current brush's shape (**Brush shape**, the
   default) or with its own round **Eraser tip** with separate strength, hardness
   and pressure settings. In both modes the eraser has its own size: with the
   Eraser selected, the size slider and **[ ]** change the eraser, and the pen
   keeps its size. Switch modes in the Eraser's panel or the brush selector. A
   pen's eraser end uses the same choice, and its dashed outline appears as soon
   as you hover with the pen flipped.
   In **Brush shape** mode, opacity, pressure, texture and stabilizer settings
   are shared with the paint brush; editing them also changes that brush.
   Choose **Eraser tip** for independent eraser settings. Tap **E** to stay on
   Eraser, or hold it for more than half a second and release to return to your
   previous tool. Tap **B** from Eraser to return to the last paint group.
   The **Stabilizer** slider smooths hand jitter; at 0 the line follows the pen
   exactly. The **Smooth / Rope** button beside it chooses how the line follows.
   Choose **Rope** there for a slow-follow stabilizer: the line trails the pen on a
   string (**Distance**, in the same menu, sets its length up to 200 screen pixels),
   holds still until the pen pulls it taut, then follows with a slight delay. It
   gives steady curves and straight runs, a guide line shows the string while you
   draw, and the stroke ends where the line is when you lift. The menu also has
   **Less smoothing on fast strokes**. These settings are saved per brush preset.
   If the line seems to trail the pen even at stabilizer 0, see
   [Pen display and graphics](#pen-display-and-graphics).
3. Draw a few strokes. **Ctrl+Z** undoes; **Ctrl+Shift+Z** redoes. These shortcuts
   act on artwork when you are drawing; text fields retain normal text editing.
4. Hold **Space** and drag to pan; scroll to zoom. **Ctrl+0** fits the canvas and
   **Ctrl+1** shows actual pixels. **H** flips the view without changing artwork.
   The checkerboard button at the right end of the status bar (**Ctrl+Shift+T**, or
   **View > Show transparency**) draws transparent areas as a checkerboard instead
   of white paper; it is remembered, and exports are unchanged.

On a smaller window, the options bar puts additional controls under **Stroke
options**, and the sidebars may hide to give the canvas more room. Use **View >
Navigator, color and layers** or **View > Tool property panel** to show a sidebar.
**Tab** hides/shows panels. **View > Automatic panel layout** restores responsive
panel choices and shows panels again after manual changes.
Above the layers, the **Color** and **Navigator** tabs share one area; click a
tab to switch, and Dudel remembers your choice. While the tool property panel
is open, color sits there instead. In very short windows, **Navigator** and
**Color** open from the inspector's top row.
In the shortest layout, **Layer** holds layer actions, blend, opacity and masks,
leaving room for the selected layer. Narrow inspectors use a draggable opacity
percentage; click it to type a value.

### Resize quickly with F

With the Brush or Eraser, press **F** and move the pen: a circle shows the size
as it changes. Click, press Enter or press F again to keep it; **Esc** puts the
old size back. Holding F and letting go also keeps it.

### Vector layers and lines

Right-click a layer, or the space below the layers, and choose **New vector
layer**. Its strokes stay editable: Dudel keeps each one as a line and redraws
the layer from them. CSP vector layers import the same way. A selection limits
drawing and erasing here too: a line keeps the part that fits inside, stays
editable, and its round end stops at the edge (scattered or mirrored lines cannot use
one). Fill and other pixel tools need a raster layer. The Eraser's
**Up to crossings** option removes a line only as far as the lines that cross it.

To color under line art, fill on a raster layer with **Stop at line centres**
turned on. The fill runs under the lines to their middles, leaving no gap. It
uses every shown vector layer, or only the chosen one when the fill reference is
a vector layer.

Use **Pointer (O)** to edit single lines. Click a line to pick it, or drag around
lines on empty canvas to lasso them; hold Shift to add to the lasso. Drag a picked
line to move it, or drag the box around the pick: corners scale from the opposite
corner, and the round handle above it rotates (Shift steps by 15°). Set **Drag a
line to: Reshape** to bend a line instead: the grabbed point
follows the pointer and the line around it follows less over the **Reach** shown
by the circle. F resizes the Reach. The Pointer panel changes the picked lines'
Size, Taper, Smooth and color; Delete removes them and Esc deselects. Each change
is one undo step. Command search lists these actions too.

## Build up the illustration

Press **G** for the bucket. **Fill to edges** is on by default and replaces the
background at soft pencil/pen borders to avoid pale seams. Use **Fill options**
in the top bar or the Fill panel to change it, adjust tolerance, close small gaps
or expand/shrink the region. For linework on a separate layer, choose **Visible
layers** or that specific layer as the reference and fill on a color layer below
it. Selections still limit the fill; mask fills use mask coverage boundaries.

**Gradients:** tap **G** again (or choose **Gradient** at the top of the Fill
panel) and drag across the canvas: the gradient starts where you press and ends
where you release, and **Shift** snaps the line to 15° steps. Pick a **Shape**
(linear, radial, angle, reflected or diamond) and **Colors**: main to secondary
color, main color to transparent, or **Custom** stops edited in the panel. A
selection limits the gradient, Lock transparency recolors only existing paint,
and each gradient is one undo step. On a mask, the colors' brightness becomes
coverage, which makes soft fade-outs. Tap **G** once more to return to the
bucket.

Use **Ctrl+Shift+N** to add a layer above the current layer: for example, separate
sketch, ink and color. Select layers in the Layers panel; double-click a name to
rename it. **Ctrl+J** duplicates the selected layer. Layer opacity and blend mode
controls sit above the list. A locked layer cannot be painted on.
The blend list has 26 modes in the usual groups (darken, lighten, contrast,
comparative and color component modes). Hover it and scroll, or press Up/Down,
to step through modes and judge each one on the canvas.

**Layer effects…** in the layer toolbar's **⋯** menu (or **Layer > Layer effects**)
opens Layer Effects for the selected layer, and a sparkle badge on a layer row marks layers with effects (click it
to edit them; it is crossed out when all are switched off). Effects include drop and
inner shadows, outer and inner glows, strokes, color/gradient/pattern overlays,
satin, and bevel and emboss. Effects follow the layer's pixels or a folder's
contents. They redraw in the background as you paint; the status bar shows when
a drawing is still catching up. Effects can be switched off, reordered or removed
and save with the project. Merge down bakes them into the merged pixels.

The **Fill** slider in Layer Effects fades the layer's own pixels while leaving
its effects visible. **Opacity** fades the whole result. Inside effects and reduced
fill share the layer's soft edge and mask, so overlays do not thicken that edge.
Gradient effects have editable color and opacity stops; pattern effects can load
an image. Layer effects and adjustments require a build that reads project v6.

**Adjustment layers:** **New adjustment layer** in the layer toolbar's **⋯** menu
(or the **Layer** menu) adds levels,
curves, brightness/contrast, exposure, hue/saturation, vibrance, color balance, black &
white, photo filter, channel mixer, selective color, gradient map, invert, posterize or
threshold above the selected layer. It recolors everything below it
(or, clipped, only its base); its opacity and mask fade it. Click its thumbnail to
open the Adjustment window: in Curves, click the graph to add a point, drag to move
it and double-click to remove it. Hue/saturation exposes individual color ranges,
gradient maps have editable middle stops, and Levels shows a histogram. The Layer
menu also provides adjustment actions.

**Paper:** **Canvas > Paper…** gives the canvas a surface that brushes feel:
Hot-pressed, Cold-pressed, Rough, Canvas, Washi or Pastel paper, or your own
mix of surface, roughness, grain size, absorbency and direction. Paint lands
on the raised parts first, so light or soft strokes break up into grain and
firm strokes cover; absorbent paper lets strokes sink in more evenly. The
dialog previews a soft and a firm stroke. Paper changes how paint is laid
down from then on (paint already there stays), is saved with the project and
can be undone. A brush's **Paper texture** setting (Brush settings > Texture)
says how much it feels the paper; erasing, smudge and blur ignore it, and
Brush Studio's test pad uses the project's paper.

**Wet paint:** a brush in **Wet** mode (the **Watercolor wash** and
**Watercolor round** presets, or Mode > Wet on any brush) paints with water.
Painting into paint that is still wet blends the two; on dry paint or paper
the stroke stays crisp. After you lift the pen, wet areas keep running
together and dry over a few seconds; that drying is its own undo step, and
water waits while you are painting. **Water** sets how long a stroke stays
wet, and Flow how fast paint builds. One layer is wet at a time, and undo
dries the canvas. **Granulation** (Brush settings > Texture) makes paint
settle into the paper's valleys, on canvases with a paper.

The paper shows on the canvas: its grain shades the canvas and shows through
paint, the way paper does, fading out as you zoom far out. Only the view
shows it; exports, thumbnails and color picks do not. **View > Paper
texture** turns it off.

**Material layers:** a layer can know what kind of paint it holds. **Layer >
New material layer** (also in the Layers panel's menus) makes one, and **Layer
> Layer material…** (or the **Material** button under the layer's blend mode)
changes the selected layer's. The dialog previews a stroke on the material.

- **Watercolor:** as a wet wash dries, water flows toward its drying edges
  and carries pigment there, so washes dry with darker rims (**Edge
  darkening**); fresh water dropped into a drying wash pushes paint aside
  into blooms (**Blooms**). Paint with a Wet brush.
- **Dry media:** every brush catches on the paper's tooth like pastel or
  charcoal, light pressure on the peaks and firm pressure into the valleys
  (**Tooth**). Without a paper on the canvas the layer brings a fine tooth of
  its own. Dry media layers hold no water.
- **Thick paint:** brushes leave paint that stands up from the canvas, with
  bristle streaks along each stroke, lit by a light you place (**Light
  from**, **Depth**, **Shine**; change them at any time). Each brush's
  **Thickness** (Brush settings > Texture) says how thick it paints; the
  eraser flattens what it removes. The **Impasto** and **Palette knife**
  presets are made for it, and **Soft pastel** for dry media.

On watercolor and thick paint layers, colors that meet mix like paint
rather than light: yellow dropped into wet blue turns green, red into blue
makes purple, white makes tints and a little black darkens strongly. This
happens wherever paint mixes: wet-on-wet, wet paint landing on wet paint,
wet paint running together as it dries, smudging and blending. On other
layers mixing stays as it was.

Clear water re-wets: a Wet brush's **Pigment** setting (beside **Water**)
says how much paint it carries, and at 0% it is clear water (the **Water
brush** preset) that makes dried paint wet again, so it runs, blooms and
dries once more, and lifts a little of it. On thick paint, a **Smudge**
brush works like a palette knife: it carries the paint's height along with
its color, scraping where it starts and piling paint where it stops; a
**Blur** brush smooths the relief.

The relief of thick paint is saved with the project and turns, flips, moves
and transforms with the paint, whether the whole layer or only a selection
moves.

When a brush setting only shows on one kind of layer (a Wet brush's water,
a brush's Thickness), the brush settings say so beside it and offer to
make the selected layer that kind; Undo changes it back. PSD
export writes the light as a clipped layer. Older Dudel refuses projects
with material layers rather than opening them without.

**Filters** change the selected layer's pixels directly, inside the selection,
with a live preview and one undo step. Their dialog sits at the canvas's top
right and leaves the canvas undimmed, so you judge the preview as it will look;
**Esc** or **Cancel** closes it without changes. The filters are **Filter > Brightness / contrast**, **Hue
/ saturation** and **Levels**; **Gaussian blur** (blurs spread into clear areas
unless Lock transparency is on); **Sharpen**, an unsharp mask with amount and
radius; and **Add noise**, in color or monochrome grain. **Filter > More
adjustments** applies any other adjustment-layer type the same way, with the
same editor as the Adjustment window: Curves, Exposure, Hue/Saturation/Luminosity,
Gradient Map, Vibrance, Color Balance, Black & White, Photo Filter, Channel Mixer,
Selective Color, Posterize and Threshold. **Invert**, **Grayscale** and extension
filters are in the same menu.

**Clipping:** the clip button in the layer action row limits a layer or adjustment
to the layer below it. The Layers panel joins clipped rows to their base with a
rail and an arrow. A clipped row reads "Clipped to" its base, and the base says
how many layers are clipped above it.

**Text:** pick the Text tool (T), then click to type on one line or drag a box
for text that wraps. Type on the canvas, then press **Done** or Ctrl+Enter, click
away or change tools; Esc cancels. Click an existing text with the Text tool to
edit it again. The options bar sets font, size, color, spacing, alignment and an
outline. With part of the text selected, bold, italic, underline, strikethrough
and color change only the selection. **Format > Fit text to box** shrinks text
that would overflow its box. Japanese and other languages type through Windows'
input methods, and Hebrew and Arabic read right to left. Each text layer keeps
one font and size. PSD export writes text as pixels.

**Pen quick menu:** press the pen's side button (or right-click) on the canvas
for a ring under the tip: your starred brushes around the outside, recent colors
inside them and four commands in the middle. Star brushes in the brush library to
add them; choose the commands in Settings > Drawing. Tap anything to use it, or
tap outside or press Esc to close.

**Masks:** the mask button in the layer toolbar (a frame around a dot) adds one in
a click: it shows everything, or only the selection when there is one, and you paint
on it at once. **Layer > Layer mask** (also in the toolbar's **⋯** menu) adds one that
hides everything. A masked layer shows its mask thumbnail beside the artwork, and an
**Artwork / Mask** row appears under the layer properties. Click the
mask thumbnail (or **Mask** in that row) to paint the mask: the
targeted thumbnail gets an accent outline and the options bar shows **Mask**. Paint
black to hide, white to reveal. **Mask actions > Show mask on canvas** (or Alt+click on the
mask thumbnail) shows the mask itself on the canvas in grays while you edit it; click
**Artwork** or the artwork thumbnail to paint the layer again. **Mask actions** has
enable, link, invert, apply and remove.

Press **L** for the Lasso: draw a shape and lift to fill it with the current
color. Its panel switches between **Fill** and **Erase** (or tap **L** again) and
between Freehand, Rectangle and Ellipse outlines, with options to constrain
proportions and draw from the center.

Press **M** for Selection or **V** for Move. The tool property panel exposes their
options. **Ctrl+D** deselects and **Ctrl+Shift+I** inverts the selection. If you enter a transform preview, finish with its
Apply action or cancel with Escape before continuing other editing operations.

For paired features such as eyes, select **one** feature on the active layer,
then choose **Transform mirrored pair…** in Selection properties. You can also
press **Ctrl+T** and enable **Mirror pair** in the transform properties or the
toolbar's **Mirror** menu. Drag the blue handles: the orange partner outline
follows with reflected movement, rotation and deformation. Both regions use
their own existing artwork; the selected eye is not copied over the other eye.
Both features must be on the active layer (or active mask).

The axis starts from the active layer's symmetry guide, or the canvas center
when no guide is configured. **Mirror axis** adjusts its center and tilt;
**Use symmetry guide** copies the guide again. Select one side of the
axis—the opposite selection is included automatically. A selection that
crosses the axis, such as a circle in the middle, splits along it; the box fits
the driving half. Hold **Ctrl** while moving to snap that half back onto the
axis so both halves meet again. Apply commits both
sides as one undo step, and Escape restores both. The driving selection remains
selected afterward. Turn **Mirror pair** off for an ordinary one-sided edit.

**G** selects Fill (tap again for gradients) and **I** selects Picker. **Alt+click** temporarily samples a
visible color. **Help > Keyboard shortcuts** lists the full bindings.

To draw from a picture, use **Layer > Add reference image…** (or the layer
toolbar's **⋯** menu). References are listed with the layers but are never
part of the artwork: painting, fill, export and undo ignore them. Use the
**Pointer (O)** to manage them: press on a reference to select and move it, drag a
corner to scale and the top handle to rotate (**Shift** snaps to 15°); pressing
empty canvas pans. Selecting a reference in the layer list switches to the
Pointer. Its panel sets opacity, above or below the artwork, flip and lock.
References are saved with the project. Drag their rows among artwork layers or onto folders to organize them.
Use **+ Add images...** in the selected reference's header to add up to 12 PNGs
to one layer. Select a thumbnail to transform that image; **Images** menu has
Arrange images and Remove image. See [reference boards](CREATOR_FEATURES.md)
for memory limits and persistence details.

The color wheel sits under the tool settings on the left; drag the bar above it to
resize. The bars between the brush library, brush settings, color, navigator and
layers all work the same way: drag to give an area more room, double-click to return it
to its automatic size. Dudel remembers the sizes. Drag the panel's edge to make it
wider, which also widens its sliders. Drag the left panel's edge all the way toward the tool rail to close the
tool settings. Clicking a tool in the rail always shows its settings, including
when a smaller window hid them automatically. Keyboard shortcuts switch tools
without opening the panel. Saving, opening and filter progress appear in the status
bar, so the canvas never shifts while they run.

**View > Workspaces** saves the current arrangement under a name (which panels
show, their widths and heights, and the Color or Navigator tab) and switches
back to it with one click: for example a painting layout with large color and
a bare one for a small screen. Saving under an existing name replaces it.

From the canvas, **F6** focuses workspace controls. **Tab** / **Shift+Tab** move
focus, and **Space** or **Enter** activates a button. **Escape** leaves a field or
dismisses the current popup. With no control focused, **Tab** hides or shows panels.

For brush editing, open the brush selector, choose **Brush settings**, then
**Open Brush Studio**. It opens as a **Brush Studio** tab beside your project
tabs and uses the whole canvas area; click a project tab to draw again, or the
Studio tab's × to close it. **Float as window** shows it over the canvas
instead (and **Open as tab** brings it back); Dudel remembers the choice.
Below the settings, a **Test pad** stays in view so you can try the brush as
you change it: it draws at true size, **Undo** takes back test strokes,
**Stripes** fills it with bands of your two colors for trying smudge and blur,
and **White**, **Gray** or **Dark** paper shows light and dark brushes. Test
strokes never touch your project. **Hide** gives the settings the room (**Show
test pad** brings it back); in a small Studio window it is the **Test pad**
section instead.
The tool property's **Basic** page keeps Mode, Size, Opacity, Stabilizer and
Follow visible. **Mode** turns any brush into a **Smudge** brush, which drags the
paint under it along the stroke like a finger, or a **Blur** brush, which softens
it; Opacity sets how strongly. The built-in **Smudge** and **Blur** presets are
ready-made ones. Both work on the layer's own paint, honor selections and Lock
transparency, and ignore Taper out; the pen's eraser end still erases. Expand **Paint & pressure** for Flow, Hardness and pressure controls;
use **Tip**, **Texture** and **Advanced** for detailed tuning.

**Brush graphs** (Brush Studio > **Brush graph**) let nodes decide each dab's
size, opacity, flow and spacing. **Make graph from current settings** turns the
brush's pressure and dynamics settings into a graph that paints exactly the
same, as a starting point, or **Start from a template** (also in the
**Templates** menu) for effects the plain settings cannot make: Downstroke ink
(thick going down, thin going up), Twisting ribbon, Dashed line, Wobbly ink,
Color jitter, Pressure color, Speed spray, Stipple and Tilt shading. Then:

- **Add node** (or right-click the canvas) adds:
  - inputs: Pressure, Speed, Tilt, Twist, Distance, Random, Constant,
    **Direction** (which way the stroke heads), **Tilt direction** (which way
    the pen leans), **Wave** (repeats along the stroke: smooth, triangle,
    dashes or ramp) and **Noise** (smooth wandering along the stroke);
  - shaping: Curve, Remap, Invert and **Steps** (snaps to a few levels);
  - combining: Multiply, Add, Minimum, Maximum, Mix;
  - outputs: Size, Opacity, Flow, Spacing, **Rotation** (turns the tip),
    **Scatter** (moves dabs off the line), and the color outputs **Hue
    shift**, **Saturation**, **Brightness** and **Color mix** (toward the
    secondary color).

  For example, Random into Hue shift gives color jitter, Pressure into Color
  mix paints the secondary color where you press hard, Wave (dashes) into
  Opacity draws a dashed line, and Speed into Scatter sprays faster strokes.
- Drag from a node's right dot to another node's left dot to wire them; drag a
  wire's end off an input to move or remove it. A wire that would make a loop
  is refused. Let a new wire go over empty canvas and a node search opens
  there: type part of a name (or of what it does), press Enter or click one,
  and the node arrives already wired. **Add node** has the same search at
  the top.
- Every node shows what it computes: its value at its output dot and as a
  bar under its title, and output nodes show theirs at the right of the
  title. **Try inputs** (below the node settings) sets the pen input they
  are shown for: pressure, speed, tilt, direction, distance along the
  stroke and random. While you draw on the test pad, the values follow your
  latest dab. Inside a group, its nodes show theirs too, with what comes in
  and goes out. Hovering a node, input or output says what it does.
- Drag nodes to arrange them, drag empty space to pan, and use the wheel to
  zoom; **Fit** shows every node. Ctrl- or Shift-click adds a node to the
  selection, and Shift-dragging empty space selects a box of them; dragging
  moves all selected nodes.
- **Formula** (under Combine) computes a value from its inputs a, b and c with
  something like `clamp(a * 2 - b, 0, 1)` or `mix(a, b, smoothstep(0, 1, c))`.
  It only accepts a formula once it is complete, and says what is wrong
  otherwise. Unwired inputs count as 0.
- **Group** packs the selected nodes into one node: values wired in become
  its inputs and values read outside become its outputs (up to eight each,
  shown as In 1… and Out 1…).
  Double-click a group or choose **Open group** to edit inside it; **Back**
  or **Main graph** returns. Rename it in its settings, or **Ungroup** to
  unpack it. Groups travel with a brush when you export it, and every group
  in your library appears under **Add node › Groups** in your other brushes.
  A graph holds up to 128 nodes, counting those inside groups.
- With the pointer over the graph, **Delete** removes the selected nodes,
  **Ctrl+Z** / **Ctrl+Y** (or **Ctrl+Shift+Z**) undo and redo graph changes,
  **Ctrl+A** selects every node, **Ctrl+G** groups, **Ctrl+Shift+G** ungroups
  and **Esc** clears the selection or leaves a group. Over the canvas these
  keys keep their usual meanings.
- Click a node to change its settings beside the canvas, such as a Curve's
  shape.
- **Undo** and **Redo** step through graph changes; **Remove graph** returns to
  the plain settings.

A connected output takes over its setting: the matching pressure checkboxes
and dynamics are greyed out while it is wired. Brushes whose graph sets color
mix each dab's color into the stroke as it is drawn, so Taper out does not
apply to them. The preview above the graph
shows the result as you work.

**Save as new** keeps a new
preset; **Update preset** replaces the selected preset after confirmation;
**Revert** returns to its saved settings.

**Sharing a brush:** **Preset > Export brush…** asks for your name, a license
(**Common licenses** fills in CC0, CC BY and others) and a short description,
then saves a `.dudelbrush` file with the brush's tip, grain and graph inside.
**Library > Import brush…** (or dropping the file on the window) shows the
brush drawing a sample stroke with its author, license and description before
**Add to library** adds it. Brush files from older Dudel (`.json`) and packs
(`.dudelbrushes`, from **Library > Export library…**) open the same way.
**Library > Add built-in brushes** adds any built-in preset your library is
missing, such as the Smudge and Blur presets.

## Save, reopen and export

**Ctrl+S** saves the editable project; the first save asks for a `.dudel` path.
Wait for **Saved** in the document header. **Ctrl+Shift+S** opens Save as for a
separate project copy. **Ctrl+O** or **File > Open recent** reopens saved work.
Layers and editable document state belong in the project file.

Save as refuses a filename owned by another open tab. Choose a different name,
or close that tab after reviewing its changes, before replacing its file.

Each save stores a small preview of the artwork in the project. On Windows,
**View > Settings > File Explorer previews > Show .dudel previews** makes File
Explorer show it as the file's thumbnail, like CSP files. It needs
`dudel_thumbnails.dll` beside `dudel.exe`, no administrator rights, and applies
to projects saved from then on. Run it again if you move the Dudel folder;
**Remove .dudel previews** returns to plain icons. Use it before deleting the
Dudel folder so Windows no longer looks for the DLL at its old location.

While a save is queued or running, **Cancel save** requests cancellation. Drawing
and undo remain available. Cancellation is checked during encoding and after the
current file flush, before publication. If publication has already passed that
fence, the save may finish and report success. Canceling a save also cancels a
waiting New/Open/Close action; the current artwork stays open.

**File > Export > Export image…** (Ctrl+Shift+E) opens a window: choose PNG,
JPEG, WebP, BMP or GIF, whether to keep transparency where the format can, JPEG
quality and the background color see-through areas become. A preview shows what
the format does before you pick a file. **Export again** (Ctrl+Shift+R) repeats the
last export to the same file with the same settings. **Export vector lines as SVG**
writes each line of the shown vector layers as a path for Illustrator or Inkscape;
raster layers, masks, effects and brush texture are left out. **Export Photoshop
document (.psd)** keeps the layers, folders, masks and adjustment layers for
Photoshop and other editors, with layer effects as editable layer styles.
**Export PSD for Live2D** draws the effects into
layers instead, since Live2D Cubism ignores layer styles.
Export creates a copy;
continue saving the `.dudel` project for later editing. **File > Import > PNG as
canvas** opens an image as a new document.

To bring images into the current document as layers, any of these work, with PNG,
JPEG, WebP, BMP and GIF files up to 16384 pixels per side:

- **Drag files onto the window.** Each image becomes a new layer above the current
  one, centered where you drop it. Dropped `.dudel`/`.clip` projects open in tabs;
  packages, brush packs, palettes and themes are imported.
- **Ctrl+V** (Edit > Paste image as layer) pastes a copied image, or every image
  file copied in File Explorer, as new layers.
- **File > Import > Images as layers** picks one or more files.

Images larger than the canvas are scaled down to fit; nothing is cropped. The new
layer's area is selected, so **Ctrl+T** transforms it straight away, and each
image is one undo step. **Edit > Paste as reference image** puts the clipboard
image on a new reference layer instead.

**File > Export > Export project package (.zip)** writes a portable copy: the exact
project, every layer and mask as a full-canvas PNG in folders that mirror your
layer folders, the reference images, a flattened preview and a `manifest.json`.
**File > Import > Project package** unpacks it into your Projects folder (never
over an existing file) and opens it. A package without `project.dudel` is
rebuilt from its layer PNGs, so layers edited in another program can come back.

If Dudel offers recovery snapshots at startup, choose **Restore copy**, inspect
the recovered drawing, then save it as a project. **View > Recover artwork**
reopens that list when snapshots are available. Recovery restores the last
completed snapshot, so regular saving is still part of the workflow. Recovery
becomes eligible after 15 seconds and waits for strokes and pending edits to
settle. Continuous drawing, active adjustments or slow storage can increase the
time between completed snapshots; it is not a guarantee to preserve every stroke.

Raster pixels and layer-mask coverage share a 2 GiB budget. About five fully
painted 100-megapixel layers fill it; smaller or sparse layers leave room for
many more. The app rejects projects beyond the supported payload budget.

## Tabs, preview window and folders

Each project opens in its own tab above the canvas. Click a tab to switch,
**Ctrl+Tab** / **Ctrl+Shift+Tab** to cycle, and **Ctrl+W**, the **×** button or a
middle-click to close one. A project that is already open is shown rather than
opened twice. Background tabs keep their artwork, undo history and view but use
no processing time; a switch waits a moment if a save, export or stroke is still
finishing. **View > Settings > Tab limit** sets how many projects can be open
(default 8); at the limit, opening replaces the current project after asking about
unsaved changes, and a limit of 1 keeps the classic single-project behavior.
Quitting asks about each project with unsaved changes by name.

**View > Preview window** opens a second window with only the artwork, to put on
another monitor, show viewers, or capture in OBS (use a Window Capture source).
Right-click it for options: fit the whole canvas or follow the main view, a
workspace, black, white or chroma-green background, a pen pointer, and always on
top. Reference layers are hidden by default. To leave out other layers, untick
**Show in preview window** in a layer's right-click menu or use **Hide layers**
in the preview's menu. Double-click or **F11** toggles full screen.

**View > Settings > Storage & folders** shows where files go. The Dudel folder
(Documents/Dudel by default) holds Projects, Exports, Packages, Themes, Brushes and
Palettes subfolders; each can point elsewhere, and file dialogs start there.
Project dialogs start beside the open project unless you turn that off. Themes
saved to the Themes folder appear in **View > Theme**. Settings, recovery
snapshots and extensions stay in the app data folder shown on the same page.

## Pen display and graphics

Enable Windows Ink in the tablet driver and choose a pressure-sensitive brush.
Open **View > Tablet input** to inspect detected pressure and native pen input.
Try light-to-heavy strokes, slow diagonals, fast curves and lifting the pen.
Mouse-like movement alone does not establish pressure support.

### If the line lags behind the pen

At stabilizer 0 Dudel places every dab exactly at the pen, in the same frame it
receives the pen input. Any remaining delay comes from how frames reach the
screen. **Low latency** (in **View > Settings > Display...**) is on by default:
frames are shown without waiting for the display's refresh (vsync) and at most
one is queued. With it off, Dudel waits for vsync and lets the graphics driver
queue up to two frames, so new ink can appear two or three refreshes after the
pen moves (roughly 35-50 ms on a 60 Hz display). On the owner's XP-Pen Artist 12
that was a visible lag, which is why low latency became the default.

If the line trails the pen, check that **Low latency** is ticked; changes apply
after restarting Dudel. If you see tearing (a brief horizontal seam while the
canvas moves) or want the GPU to work less while you draw, untick it. Dudel keeps
your choice.

The same window selects the graphics API: **OpenGL** (default, most compatible),
**Automatic** (DirectX 12 or Vulkan), **Vulkan** or **DirectX 12**. Both choices
apply after restarting; if a chosen API cannot start, Dudel falls back to OpenGL.
**View > Settings > Performance** shows CPU frame and drawing times; it does not
measure pen-to-screen latency.
Supported CSP import remains limited; unsupported content is rejected rather
than treated as a fully compatible editable document.

Build/run instructions are in the [README](../README.md); planned work is in the
[roadmap](dev/ROADMAP.md).

See [file format support](FORMAT_SUPPORT.md) for the native-version and restricted CSP import matrix.

## Timelapse and personalization

Use View > Timelapse to record artwork and export an animated GIF or keep its PNG frames. View > Theme selects or imports a persistent UI palette. Reference rows have separate visibility/lock controls and Move up/down actions. See [creator features](CREATOR_FEATURES.md) for recording limits, reference organization and the new brush presets.

ZIP packages from this build also preserve layer masks (including enabled/link
settings), locks, guides and reference-board placements in the manifest. When
rebuilding from edited PNGs, keep their canvas dimensions and the manifest paths.
Keep project.dudel for an exact project restore, including history-independent
project data that is not represented by editable layer PNGs. Older packages
without reference placement metadata require their embedded project.dudel.

In Rope mode, **Distance** sets rope length from 0 to 200 screen pixels,
independently of follow smoothing. It appears in the toolbar's Smooth / Rope menu,
brush settings, Stroke options and Brush Studio. Save/update the brush preset to keep the setting. Older brush
presets retain their former rope length until you adjust it. A painting contact
can start in the empty canvas surround and travel off the page and back; paint
is clipped to document pixels, and the whole contact remains one undo action.

Layer rows show a symmetry badge for their own settings, a globe for inherited
global symmetry, and a ruler badge for guides belonging to that layer. Click a
badge or **Guides** to edit the layer, assign guides, add/remove guides,
or select the layer and move its handles with Pointer. An Off layer override
disables symmetry there while keeping global symmetry on other layers.
The **Global** control above the layer list edits shared symmetry independently
of layer overrides. Global guide lines, rulers and the grid have their own count.

View > Performance distinguishes active-project allocations from tracked CPU
allocations across every open project, including parked tabs and references.
Canvas/reference/thumbnail RGBA estimates are separate; these are diagnostic
estimates, not total process memory or committed graphics memory. The window
scrolls when needed. Leave it closed during normal performance measurements.

Large canvases now retain more detail near the visible area and use a smaller
representation elsewhere. Detail returns as you pan or zoom; saved artwork and
exports keep their original resolution. Properties > Studio opens the section
you were adjusting. View > Performance includes recent latency percentiles and
cache statistics for diagnosing pauses while drawing.
