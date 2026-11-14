<!-- SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com> -->
<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->

> **DRAFT — FOR COUNSEL REVIEW.** This additional term has not yet been reviewed by a lawyer. It is
> included in this private preview so reviewers can see the intended terms. Do not rely on it as final
> wording. (XIAR plan §4A.8.)

```text
XIAR Platform: Additional Terms under Section 7 of the
GNU Affero General Public License, version 3 (AGPLv3)

Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) and XIAR contributors
XIAR - https://xiarv0.com

The XIAR platform is licensed under the GNU Affero General Public License,
version 3 or (at your option) any later version ("AGPL-3.0-or-later"), see
the file LICENSE. As permitted by section 7(b) of the AGPLv3, the copyright
holders of the XIAR platform add the following additional term to the parts
of the Program for which they hold copyright. This additional term does not
apply to third-party components that are distributed with the Program under
their own licences.

1. Attribution notice.
   If you convey the Program or a work based on the Program, or if you let
   users interact with it remotely through a computer network (section 13),
   you must preserve the following author attribution in the "Appropriate
   Legal Notices" (as defined in section 0) displayed by its user interface:

       Powered by XIAR V0 - https://xiarv0.com

2. Reasonable placement.
   The attribution must be visible and legible in at least one location
   where users would normally find the Appropriate Legal Notices, such as:
     (a) the "About", "Legal" or "Credits" screen or dialog of a desktop,
         mobile or kiosk interface;
     (b) the footer, or an "About"/"Legal" page linked from every page or
         from the main menu, of a web interface or hosted service;
     (c) for voice-only interfaces, the response to a spoken request for
         "about", "credits" or "legal notices"; and
     (d) for interfaces without any display or such command, the
         accompanying documentation.
   The attribution may use the interface's normal font, size and styling.
   It does not need to appear on every screen, in marketing material, in
   output produced by the Program, or in a prominence greater than the other
   Appropriate Legal Notices.

3. Link.
   Where the interface can display hyperlinks, the words "XIAR" or the
   address "https://xiarv0.com" must link to https://xiarv0.com.

4. Modified versions.
   You may add your own name or brand next to the attribution (for example
   "YourProduct - Powered by XIAR V0 - https://xiarv0.com"). If your version is
   modified, you may describe it as "based on XIAR". You must not remove or
   obscure the attribution.

5. No other restriction; no trademark licence.
   This additional term does not restrict running, studying, modifying or
   sharing the Program beyond the attribution above, and it does not grant
   any right to use the XIAR name or logos except to give this attribution.
   Use of the XIAR trademarks is governed by the XIAR trademark policy at
   https://xiarv0.com/trademark. Nothing here implies endorsement by the
   XIAR project.

6. Interpretation.
   This additional term is intended to be a term permitted by section 7(b)
   of the AGPLv3 and shall be interpreted accordingly. If any part of it is
   found not to be permitted by section 7, only that part shall be treated
   as removed.
```

**Scope in this repository:** the term covers the XIAR-authored code and docs. It does **not** cover
third-party components under their own licences, in particular `core/rust/prompts/mirix/` (MIRIX,
Apache-2.0), `core/rust/vendor/xiar-qdrant-edge/` (Apache-2.0) and
`core/neural-brain/tokenizers/mistral-v0.1/` (Apache-2.0).

**Open points for counsel** (plan §4A.8): items 1–3 within 7(b) (link and hosted footer); interaction with
"-or-later"; passing the term downstream; OSI/distribution acceptance; optional 7(c)/7(e) companion terms.
