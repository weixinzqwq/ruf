# Launcher send bridge

This is a separate candidate for the official `aola.100bt.com/play/start~2107300001.swf` resource. The normal Aola player path does not install it; the launcher mode installs it through `AolaNavigator` and preserves the requested response URL. Other resources remain on the existing backend.

The original ZWS was copied from the local Aola source snapshot. Its SHA256 is `51cf285ad9800adce78da174191d1ecfb676669c8ce403c0cbef6b95a24e9d72`; the decompressed FWS SHA256 is `f9733de901b0e62526d22b7d11cc76c42095c98fc1c79fdf02508b90870ad094`. No original was overwritten and no online game request was sent during preparation.

`Load.as` preserves the original version check, loading text, loader resource selection, same-domain `loadBytes` and startup event lifecycle. Its only constructor addition registers two callbacks:

- `aolaBridgeReady()` returns `AolaLauncherBridge1` only once the game's `ActClientSender` class is available, otherwise `BridgeNotReady`.
- `aolaSendCommand(requestId, extensionName, command, parametersJson)` calls the existing `ActClientSender.sendMessage(..., false)`. The game owns socket selection, sequence numbers and encryption. No new connection is created by this adapter.

The game callback reports `aolaCommandCompleted(requestId, status, code)` without response bodies. `SubmittedToFlash` only means submission. `ServerResponse` means the callback had no recognized numeric result field; it is not a success result. The `r`, `suc`, `res` interpretation follows the existing host contract. Rust correlates request IDs with the current session.

The bridge has no theme, panel, playback, GC or battle code. Rust rejects native auto command `13/1409` in phase 1 because the reference host sends it through its battle-specific bridge.

Rebuild from the checked-in original FWS and `Load.as` with `tools/build-launcher-bridge.ps1` (FFDec and Python required). It compiles to an independent candidate, rereads scripts and SymbolClass with FFDec, checks that only DoABC changed, then updates the candidate. All 11 tag positions and the frame header are preserved; tag 7 (DoABC) is the only changed tag. `metadata/verify.json` and `metadata/tag-comparison.json` retain structural verification. The Rust tests additionally load the original and candidate with a null network backend, check callback registration and verify rejection before the game sender is available.

Static export and offline registration tests do not establish online compatibility. Official login, existing-session send, server response and subsequent normal game operations still require manual user acceptance. Keep the existing no-tools launcher available for comparison.

## Offline send fixture

`fixtures/start-with-mock.swf` inserts an additional mock `ActClientSender` DoABC before the actual candidate DoABC, without changing the bridge bytecode. It is referenced only by Rust tests and is never installed as a game resource. The mock reports the arguments and invokes the callback with success, rejection or an unspecified result; it has no socket/network code. The test verifies extension mapping, UTF-8/JSON forwarding, removal of `:ext_seq;`, callback request correlation and the distinction between submission and response results.

`fixtures/ActClientSender.abc` was compiled from the neighboring `.as` using the existing Ruffle `engine/tools/asc/asc.jar` and a local `playerglobal_import.abc` build output. To rebuild the ABC, invoke Java with `-classpath engine/tools/asc/asc.jar macromedia.asc.embedding.Main -AS3 -import <playerglobal_import.abc> source/launcher-bridge/fixtures/ActClientSender.as`. `fixtures/embed-mock.py` regenerates the test SWF after rebuilding the production candidate.
