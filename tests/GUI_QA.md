# Native Windows GUI verification

The final GUI evidence is `verification/gui-v2.json`. Its screenshots and all
final assertions were captured from one immutable copy of the release EXE;
the SHA-256 is recorded in that report. Fixture media is not included here.

`gui_ui_driver.c` is a small native Win32 test driver. It is not part of the
shipping application. Build it as a **64-bit** console program, for example:

```sh
x86_64-w64-mingw32-clang -municode gui_ui_driver.c -o gui_ui_driver.exe \
  -luser32 -lgdi32 -lcomctl32 -lshell32 -static
```

Run the tested application on an interactive Windows desktop, then run the
driver in the same session. Under Wine, use the same prefix and display as
the application. Useful commands are:

```text
gui_ui_driver.exe inspect
gui_ui_driver.exe rows
gui_ui_driver.exe command 116
gui_ui_driver.exe command 114
gui_ui_driver.exe dialoginspect
gui_ui_driver.exe dialogbutton 601
gui_ui_driver.exe command 101
gui_ui_driver.exe dialoginput "C:\test-inputs"
gui_ui_driver.exe dialoginput "\"first.png\" \"second.heic\""
gui_ui_driver.exe choose 117 2
gui_ui_driver.exe choose 118 3
gui_ui_driver.exe command 106
gui_ui_driver.exe command 107
```

Button IDs: Add 101; Remove 102; Clear 103; Destination 104; Reset destination
105; Convert 106; Cancel 107; Open output 108; Keep metadata 109; About 114;
Language 116. Image selector 117 uses HEIC/PNG/JPG/WebP indices 0/1/2/3;
video selector 118 uses MP4/MOV/AVI/WebM indices 0/1/2/3. Use `click 109`
to exercise the checkbox itself, including its native checked state.

The `drop` command accepts one or more absolute Windows file paths. It writes
a bounded `DROPFILES` payload into read/write memory in the application
process and sends the native `WM_DROPFILES` message. It does not write or
execute code. The command waits until the application's real modal closes;
use the modal buttons manually or, from a second terminal, run
`dialogbutton 501` to review the queue or `dialogbutton 502` to convert now.
The `rows` command similarly reads native ListView text using a bounded
data buffer. This verifies the actual handlers and controls; it does not
simulate a physical Explorer mouse gesture.

The recorded cancellation check used a 1280×720, 30-second MP4 with WebM
selected. It waited for real progress above the initialization value,
canceled, verified that the row was retryable and its output folder had no
partial or published file, then retried the same row with the same settings.
After a second safe cancel, selecting MP4 completed the existing queue row
with dimensions and source bytes preserved. The same execution also checked
that format changes requeue the affected media, metadata changes requeue
completed rows, and Persian mode translates the common warnings and errors.

When using a synchronized remote execution environment, check temporary-file
absence inside the same live test command. Later commands observed original
zero-byte reservations and runtime files reappearing after cleanup assertions
had passed. This is consistent with executor/Wine filesystem synchronization;
that attribution is an inference, not a proven cause. The report distinguishes
these observations from the application's observed cleanup.

`capture <absolute-bmp-path>` captures the current visible native client
region directly with GDI. The preview PNG is a lossless format encoding of
this native bitmap, without cropping or image edits.
