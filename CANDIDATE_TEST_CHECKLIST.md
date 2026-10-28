# Candidate workflow check

This is a reference for targeted diagnosis, not a standing request to repeat the
whole checklist. On 2026-10-28 the owner confirmed working Artist 12 use across
development testing and working Explorer previews. Reuse that feedback and the
completed performance studies. Request only a specific check that answers a
concrete unresolved question; explain why existing evidence is insufficient.

Allow about 20-30 minutes using disposable projects or copies of your artwork.
Record the candidate folder, Windows version, Artist 12 driver and display scaling.
Use BUILD.json if reporting an exact executable hash.

1. **Projects and symmetry:** create two projects with different global symmetry
   and snapping settings. Switch back and forth with Ctrl+Tab. Each project should
   keep its settings, and switching alone should not mark a saved project changed.
   Cancel an unsaved close. Try Save as onto the other open project's filename:
   it should refuse; saving to a different name should work.
2. **Pen and eraser:** draw light-to-heavy and fast curved strokes, then switch
   between Pencil/Pen and Eraser. Check that each remembered size feels correct.
   With Rope enabled, draw past a canvas edge and back; the stroke should continue
   following your pen while outside the page.
3. **Coloring and adjustments:** fill inside soft linework with Fill to edges on.
   Check for pale seams and that Undo restores the previous pixels. Drag a
   Brightness/Contrast slider repeatedly; the dialog should stay stationary.
   Try both Cancel and Apply.
4. **Themes:** edit Paper's text, accent and Outside canvas color repeatedly.
   Verify the pickers remain interactive. If contrast becomes invalid, use
   Restore readable colors, then Apply and Save theme. Restart and check the
   applied theme. The preview window's Workspace background should follow the
   customized outside-canvas color.
5. **Layers and references:** organize reference images into a layer folder,
   reorder it among artwork layers and select individual reference images.
   Add a mask, paint on it, perform a selection/transform and Undo/Redo. Check
   that the active layer/mask and symmetry/guide indicators remain understandable.
   Transform a Multiply or clipped layer beneath visible artwork: during the
   drag its completed preview should keep the layer's blend and occlusion.
   Apply/Undo and Cancel should restore the expected artwork. For larger complex
   layers, handles may lead the latest completed background preview.
6. **Save and share:** save, close and reopen the project. Compare artwork,
   masks, folders, references and guide settings. Export opaque and transparent
   PNGs; export should preserve editable project state. Try a project ZIP round
   trip on a disposable copy.
7. **Recent drawing changes:** on a 4096-square canvas, draw small and large strokes,
   pan/zoom and rotate across the artwork, then check the navigator. Repeat with
   timelapse on and off. Fill a large region, then Undo/Redo. Report visible stalls,
   stale regions or unexpected loss of detail; do not infer GPU memory usage from
   the texture-byte estimate alone.
8. **Mirrored feature editing:** put two distinct marks on one layer, select one,
   and use Selection properties > Transform mirrored pair. Move/rotate, Apply,
   Undo/Redo and Cancel. Each side should keep its own artwork and move symmetrically.
9. **Explorer previews (Windows):** save a disposable project, enable View > Settings >
   File Explorer previews > Show .dudel previews, and inspect it in a thumbnail view.
   Save a visibly different image and refresh Explorer. Remove the preview registration
   before deleting the test app folder. If testing an upgrade or moved folder,
   re-enable previews from the new location and record the result.

For each issue, report the step, what happened, what you expected and whether it
repeats after reopening the app. Include a screenshot or a disposable example
project when useful. A completed checklist supplies workflow feedback; the
two-hour tablet session, unattended DX12 study and clean-machine acceptance remain
separate release checks.
