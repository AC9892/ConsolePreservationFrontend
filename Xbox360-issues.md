# Xbox 360 dashboard issues and next work

Updated: 2026-10-30. Scope: authentic Metro dashboard **2.0.17559.0** running in the CPF-modified Xenia Canary backend.

Source: [Xbox360 branch](https://github.com/AC9892/ConsolePreservationFrontend/tree/Xbox360), published checkpoint [`539423b`](https://github.com/AC9892/ConsolePreservationFrontend/commit/539423b5352f2f94a7a2f1a1d0d4f8fccc8836dd). The source-only gamer-tile and default-image API experiments are published as additive patches. Earlier local Game Details diagnostics are not automatically included. No release accompanies this report.

Status: **Local runtime confirmed** = directly observed in the development candidate; **User reported** = manual observation needing a current regression test; **Open verification** = not demonstrated; **Incomplete implementation** = missing implementation or lifecycle contract. A queued request, successful build or mounted image does not prove a working page or gameplay.

## Priority problem tree

```text
Authentic dashboard functionality
|-- 1. Game Details does not open
|   |-- Correct original scene is queued; Home remains visible
|   |-- Inactive-demand output correction alone did not resolve it
|   `-- Next: trace route consumption and back-URI handling
|-- 2. Game execution and return path
|   |-- Restart-based launch reaches one game's welcome/setup screen
|   |-- Gameplay and wider game compatibility remain unverified
|   `-- Seamless dashboard -> game -> dashboard requires lifecycle work
|-- 3. Original system functionality
|   |-- Personalized avatars and customization incomplete
|   |-- Preferences and several profile/social actions incomplete
|   `-- Remaining DASHUSER resource failures need causality checks
`-- 4. Validation and publication
    |-- Profile persistence and system-app regression tests
    `-- Publish only reviewed source/documentation after validation
```

## Problems, evidence and completion criteria

| ID | Status | Evidence / limitation | Next work and completion criterion |
| --- | --- | --- | --- |
| X360-DETAILS-01 | **Local runtime confirmed** | Mounting a disc exposes Play Game and the Game Details prompt. Requesting details queues the corrected original scene once, but Home stays visible. | Trace the guest's route consumption and first failing prerequisite. Success requires the authentic details page to render, accept input and return to Home. |
| X360-DEMAND-01 | **Partial local correction; broader implementation incomplete** | An original query writes two nullable outputs even when inactive; a dashboard caller consumes one output without checking the flag. Local gated handling now supplies the observed inactive values and removes its undefined-call warning. Game Details still fails. | Keep inactive-query behavior separate from active-demand support. Determine actual state/set/complete contracts before implementing active requests. Local correction is not in the published checkpoint. |
| X360-BACKURI-01 | **Incomplete implementation** | PushBackURI remains undefined during the failing transition. Original implementation stores bounded URI state; a success-only stub would omit behavior. The retail debug getter export examined has no callable entry. This is a candidate dependency, not a proven sole cause. | Trace producers, consumers, clearing and return navigation; implement only established semantics. Do not invent prior-title state or filesystem aliases from warnings alone. |
| X360-LIFE-01 | **Incomplete implementation** | Seamless same-process title switching has not been established. Hotkey callback/shutdown safeguards are only one part of backend retirement. | Establish guest-thread completion, GPU/APU quiescence, callback cancellation and UI-thread progress before resetting title resources. Verify repeated dashboard/game/return cycles without stale callbacks, hangs or save loss. Do not bypass launch guards. |
| X360-GAME-01 | **Open verification** | A restart-based handoff reached Halo: Reach's original welcome/setup screen; selection movement was manually confirmed. Gameplay was not established. | Test progression into gameplay, input, save behavior and the supported return path using working copies. Broaden to other owned games only after a reproducible baseline. |
| X360-AVATAR-01 | **Incomplete; locally investigated** | A controlled initialization-on test still shows a gray figure and reaches zero-size/incomplete asset handlers. Original Metro wrappers use the F2 service. Candidate pack indexes were bounded successfully, but asset identifiers and guest/GPU output layouts remain unresolved. | Implement authentic original asset/animation loading only after the ABI is established; verify personalized rendering and customization persistence. Do not substitute characters or success-only stubs. |
| X360-SETTINGS-01 | **User reported; retest needed** | Preferences does not open correctly. Profile settings invokes Xenia's profile editor rather than the original console page. | Reproduce each action separately and trace missing system-app/UI handling. Success means the original page works and returns normally. |
| X360-SOCIAL-01 | **User reported; incomplete** | Friends, Activity Feed and Avatar Store invoke Xenia's sign-in UI instead of providing original functionality. | Separate local sign-in behavior from network/service dependencies. Opening sign-in is not proof these features work. |
| X360-DASHUSER-01 | **Observed resource failures; impact unproven** | DASHUSER mounts with a profile, but two early resource lookups remain unresolved, including retries. | Identify requested resources and compare lookup timing with visible failures. Do not assume they cause avatar or details problems without evidence. |
| X360-PROFILE-01 | **Partial / open verification** | Existing test profile loads; creation and selection have worked partially. Reliable settings/sign-in persistence across repeated launches is not fully established. | Reuse the existing profile; test selection, local sign-in/out, persistence and normal shutdown without creating unnecessary profiles. |
| X360-LIBRARY-01 | **Partial / open verification** | My Games opens empty; My Apps shows six original entries. Recents opens, but history accuracy is unverified. A mounted disc is not automatically a library installation. | Test enumeration and supported original app launch/return separately. Verify Recents against actual supported launches. |
| X360-REGRESSION-01 | **Open verification** | Most system apps, nested settings, additional themes, other games and long-session stability lack complete validation. | Maintain a per-feature pass/fail/untested matrix. Publish claims only for the exact tested build. |

## Current picture and avatar checkpoint (2026-10-30)

| ID | Status | Evidence / limitation | Next work and completion criterion |
| --- | --- | --- | --- |
| X360-PICTURE-01 | **Stored-picture rendering confirmed with synthetic input** | Original dashboard 12581 displayed synthetic 32/64 PNGs in a disposable profile copy. The working profile had no picture files or avatar manifest. This is not authentic picture selection/save or persistence validation, and does not prove the changed texture-copy function was reached. | Test a genuine selected picture through the existing save flow, both resolutions and a persistence relaunch; preserve working profiles. |
| X360-PICTURE-02 | **Published implementation; visual benefit unconfirmed** | Gamer tile conversion now honors row pitch, rejects insufficient extents and handles failed decoding. Build passed; two synthetic tests passed with eight assertions. | Verify the modified texture-copy path with real dashboard callers; retain the distinction between helper tests and dashboard behavior. |
| X360-DEFAULTIMAGE-01 | **Published opt-in experiment; incomplete verification** | A bounded 64x64 PNG loader and XAM-owned guest cache implement the observed pointer/optional-size contract. Candidate boots original Metro 17559, renders Home/Social and accepts shoulder navigation. Undefined default-image call warning is absent. Successful cache return and a distinct visible picture improvement were not established. | Verify invocation/result, missing/invalid inputs, concurrent calls and teardown. Original default image must be supplied privately; no resource is distributed. |
| X360-HISTORICAL-01 | **Limited runtime confirmed** | Original 1888, 7371 and 12581 dashboards render and respond to keyboard navigation in isolated setups. Supplied NXE XAM patch rejects its base; initial NXE rendering still works. | Broaden profile/settings tests without implying complete compatibility. Metro 17559 remains the primary target. |

No personalized 3D avatar success, full picture-selection workflow or new gameplay support is claimed. New default-image configuration is empty by default; original dashboard resources, profile data and logs remain local. No release was created.

## Earlier baseline and Game Details test (2026-10-29)

- Authentic 17559 dashboard boots and renders with the existing test profile.
- Controller/keyboard input and section navigation work; Settings, Family Settings, achievements and theme-selection interfaces have been reached in prior tests/reports. Alternate themes themselves are untested.
- Read-only disc mount/eject works and Home shows Play Game.
- Latest demand-query candidate: Home rendered; disc mounted; details request queued once; Home remained; Social navigation still responded; normal close succeeded with no hotkey-worker shutdown failure.
- Latest candidate was built with Visual Studio 2022 using `/Z7`. Existing kernel tests passed **8 cases / 288 assertions**; these are not Game Details or demand-query behavioral tests.
- No new build was needed for the retained-candidate runtime test. No full gameplay, personalized-avatar or seamless-switching claim is made.

## Work order

Current focus: Metro 17559 gamer pictures and authentic 3D avatar asset service, including the verification criteria above. The existing dashboard backlog remains:

1. Trace and fix Game Details using the first established failing dependency; perform one focused regression run.
2. Validate restart-based game execution further; address lifecycle prerequisites before seamless switching.
3. Investigate original avatars, Preferences and system-app handling.
4. Verify profile persistence, libraries and wider compatibility.
5. Update public source and documentation after review; keep development-only fixes clearly labeled.

This issue report contains sanitized findings only. Firmware, dashboard executables/assets, game images, keys, profiles, raw logs, disassembly and console-specific data remain private and are not required to read this report.
