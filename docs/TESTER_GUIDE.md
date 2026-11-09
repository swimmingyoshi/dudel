# Try Dudel: a 30-minute test

Thanks for trying Dudel. It's a free, open-source drawing app in early
pre-release, and so far only its developer has drawn with it. What we need most
is to hear how it feels to someone else, on a different tablet and computer.

You don't need to be technical, and "this felt wrong" is useful feedback.

## Before you start

- **Protect your art.** This is a pre-release. Use it for test drawings, and open
  *copies* of any files you care about.
- **Privacy.** Dudel has no telemetry and no update check, and it doesn't send
  anything anywhere. Nothing is shared unless you post it yourself.
- **Windows warning.** Downloads aren't code-signed yet, so Windows may say
  "Windows protected your PC". Choose **More info > Run anyway**.

## Install

Download from the [Releases page](https://github.com/swimmingyoshi/dudel/releases).

**Windows:** get `dudel-<version>-windows-x64.zip`, extract the **whole** ZIP and
run `dudel.exe`. There is no installer. To remove it, delete the folder; your
projects stay in `Documents\Dudel`.

**Linux (experimental):** get `dudel-<version>-linux-x64.tar.gz`, extract it and
run `./dudel`. Pen pressure needs a Wayland session.

## The test

1. **Check your pen (2 minutes).** Open **View > Tablet input**, then draw light
   to heavy strokes. On Windows, your tablet driver needs Windows Ink turned on.
   Does the line get thicker and darker as you press? Does the eraser end erase?
2. **Draw something real (15 minutes).** Press `Ctrl+N` for a new canvas and draw
   what you normally would: a sketch, a character or a study. Use layers, undo,
   zoom and rotate, and try a few brushes. Press `Ctrl+K` to search for any
   command you can't find.
3. **Notice what's missing.** When you reach for something and it isn't there,
   write it down. That's one of the most useful things you can tell us.
4. **Save and reopen (3 minutes).** Press `Ctrl+S`, close Dudel, open it again and
   open your project. Then use **File > Export > Export PNG** and look at the image.
5. **Optional: bring in your own work (5 minutes).** Press `Ctrl+O` and open a
   *copy* of a `.psd` or Clip Studio `.clip` file. Does it look right? Are the
   layers still there?

## Tell us how it went

[Fill in the feedback form](https://github.com/swimmingyoshi/dudel/issues/new?template=tester_feedback.yml).
It takes about five minutes and needs a free GitHub account. Your answers are
public, so leave out anything private. Only attach art you're happy to share.

For a reproducible bug, use the
[bug report](https://github.com/swimmingyoshi/dudel/issues/new?template=bug_report.md)
instead. For questions and ideas, use
[Discussions](https://github.com/swimmingyoshi/dudel/discussions).

The [quick start](QUICK_START.md) covers everything in more detail.
