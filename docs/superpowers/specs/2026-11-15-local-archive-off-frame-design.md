# A local Protocol archive unpacks off the frame

## Intent

A Protocol archive chosen on this Mac (`⌘O`, a drop, the picker) unpacks on the UI thread, so one near the 2 GiB limit holds the window for as long as the unpack takes. A link's archive already downloads and unpacks on a worker with a progress banner. A local archive should do the same.

## Goals

- A local archive's unpack runs on the worker a link's download uses, with a banner naming the archive while it unpacks, and frames keep drawing meanwhile.
- When it lands it opens as a link's Protocol does: restricted unless a folder above it is trusted, with the close question first over a draft, and its origin noted as "the archive <name>".
- A refused archive says why under "Could not open that archive", as today.
- An archive handed over among other files raises "Only the Protocol was opened" once the unpack lands, and not when it is refused.

## Non-goals

- Moving a local folder or workbook's open off the frame.
- A byte-progress readout for the unpack.
- Running two archive jobs at once.

## Approaches

1. **Start the unpack where the open is asked for, as a link does, and open the result through the open queue's route when it lands (chosen).** The close question is asked once, after the unpack, over whatever draft the window then holds; nothing is left until the Protocol is ready.
2. Keep the unpack where it runs today, after the close question was answered, and open the result directly when it lands. Rejected: an edit made while the unpack runs would be opened over without a question, and routing the result through the queue instead would ask a second time over the draft just answered, since closing without saving does not clear the draft.
3. Unpack on a fresh, separate worker type. Rejected: the link's `Download` already carries the outcome channel and wake-up the window polls; one job slot keeps "one at a time" true for both.

## Design

- `protocol_archive::Download` gains a constructor for a local archive that runs `unpack` on its worker, and says which kind it is, so the window words the banner, the origin and a failure for each.
- The window starts a local archive's job when the request is taken (a single archive path, or a set of paths whose first Protocol is an archive), before anything is left, and from the picker's own open. While any job runs, another archive or link is refused with a banner saying what is still in progress.
- On landing: the origin is noted, the Protocol folder is opened through the same route a link's is, and the "Only the Protocol was opened" banner is raised when the job carried other files.

## Left to the builder

The banner's wording; the test hooks.
