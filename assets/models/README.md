# Emergency simulation models

Presentation proportions are centralized in `crates/game/src/visual_scale.rs`:
building footprints ×1.9 and heights ×2.5, cars ×3, people ×4.5, emergency
units ×7, trees ×1.6 and shrubs ×1.25. Vegetation multipliers apply to the
existing species sizes. These affect rendering only; scenario coordinates,
terrain and fire resolution, routes, plant counts and mesh detail are unchanged.

Original, editable Blender assets generated for this project; no external art or textures (photographic references for the plants: `CREDITS.txt`).

- `emergency_assets.blend`: ten named collections (pedestrian, firefighter, car, fire_engine, pine, oak, chestnut, bush, olive, cypress). Each collection is authored at the origin; isolate a collection to edit it.
- `meshes.json`: baked positions, normals, vertex colors, triangle indices and bark masks, embedded in the native and web game.
- `preview.png`: Blender studio render. The fire engine is shown with red paint; game symbols use their live operational status tint.

- `units.json`, `units_assets.blend`, `units_preview.png`: the player's teams, from `scripts/build_unit_models.py` (firefighter on foot, engine, air tanker). Real colours baked as vertex colours (red engine with blue beacons, orange AIB jackets and white helmets, white aircraft with red markings); the game no longer tints them by team or status (only a lost unit goes dark and a withdrawing one reddish). Animated parts are separate entries `<unit>.<part>` with a `rig` (pivot, axis, kind): engine wheels and beacons, firefighter legs and arms (the tool arm chops while working), tanker propellers; `units::animate_units` drives them on real time. Regenerate with `/Applications/Blender.app/Contents/MacOS/Blender --background --python scripts/build_unit_models.py`.

- `vegetation.json`, `foliage_atlas.png`, `vegetation_assets.blend`, `vegetation_preview/` (open `index.html`): the plants of the landscape, from `scripts/build_vegetation_models.py` (remodelled 2026-10-31). Fourteen Mediterranean species, each with 2–4 **varieties** (stone pine, Aleppo/maritime pine, cypress; holm oak, cork oak, carob, olive; lentisk, myrtle, strawberry tree, broom, tree heath, rosemary, cistus), placed by fuel class in `vegetation.rs` (`species_for`): the species of a fuel group share its outline, so what is drawn is what burns. A variety's **live** model is a four-sided trunk and limbs with bark mapped on them, and per crown part a lumpy 12-vertex core (the canopy tile projected from above) plus 2–3 **leaf cards** cut out of the atlas by its alpha; normals are authored (a card lights as part of its crown volume) and `uvs` point into `foliage_atlas.png`. The atlas (1024 × 512, sRGB) is baked by the same script: leaf sprays rendered with Workbench from leaf geometry (needle tufts, scale sprays, leaves with two-tone undersides, rods, heath twigs, flowers and berries), opaque 64 px canopy tiles for the distant proxies, bark strips from Blender's procedural noise, and a flat grey for what is untextured. The game multiplies the atlas by a per-plant tint (vertex colour × 2). **What fire adds** (`remains`: twig skeleton and debris; `low`: 5 triangles far away) is as before, untextured. `canopy`/`bark` give each variety's proxy texture. Budget (asserted by `models::the_vegetation_varieties_are_within_budget`): live ≤ 120 triangles / 96 vertices, remains ≤ 120 / 160. Visual references and licences: `CREDITS.txt`. Regenerate with `/Applications/Blender.app/Contents/MacOS/Blender --background --python scripts/build_vegetation_models.py` (under a minute; `-- --atlas-only` for the atlas, `-- --no-preview` without the Cycles overview).

- `rocca_scans.json`: the rocca for the realistic look only (`KIOSK_SCANS=1`), from `scripts/build_rocca_real.py` (2026-11-06): the rocca of `island.json` built by `scripts/build_island_models.py`'s own code with the same random draws (the crag under it is that bake's `rocca_roccia`, checked by the script), but with walls of flat-topped columns stepping down course by course where they are broken, wall-walks with parapets on corbels and swallow-tail merlons where they stand whole, openings cut through the walls (exact boolean) with dressed jambs, voussoirs and sills, the gate as a round arch with the wall over it, and each stretch of wall lifted onto the crag whole instead of sheared over its beds. Not in the web data (`build_web.sh` leaves out `_scans`): there the island bake's rocca is drawn. Regenerate with `/Applications/Blender.app/Contents/MacOS/Blender --background --python scripts/build_rocca_real.py` (about a minute).

Regenerate from the repository root:

```sh
/Applications/Blender.app/Contents/MacOS/Blender --background --python scripts/build_models.py
cargo test -p game models::tests
```

The generator is the source of truth: regeneration rebuilds the `.blend` and mesh bake. Make persistent design changes in `scripts/build_models.py`.

Blender coordinates are metres, Z up, front -Y; baked meshes use Y up and front +Z. People and vehicles have their origins at ground level. Trees include maritime pines (tall bare leaning trunk, flat umbrella pads), chestnuts (short trunk, big lobed round crown), broad oaks, gnarled olives, and narrow cypresses, and are approximately one metre tall before the vegetation system applies species-specific sizes. Bushes are normalized clumps. Existing map-symbol scale factors still apply.

People and suppression models bake neutral vertex tones so the existing status materials stay legible. Cars retain colored details. Vegetation retains species palettes and authored crown shading; bark masks distinguish stems from leaves. Plant meshes are welded, about 40–50 vertices / 72 triangles per archetype (lumpy 12-vertex crowns; the foliage pixel shader adds the texture), and merged into existing spatial chunks so burning and culling keep working. Grass remains procedural.

Triangle budgets: pedestrian 356, firefighter 388, car 712, fire engine 1,688, pine 112, oak 152, chestnut 156, bush 100, olive 144, cypress 52. The landscape uses pine (fuel 10-12), chestnut (4-6) and bush (7-9) only, one model per group, so the fuel reads from the silhouette; olives remain as lone trees in grassland. More detailed vegetation increases triangle counts versus the former procedural models; draw-call batching and the existing browser density reduction are preserved.

Rocca Ventosa's hills, field parcels, curving village streets and rural lanes are
baked by `scripts/generate_demo_scenarios.py demo_borgo`. Isolated homes are real
populated buildings connected to the evacuation road network. Class 4 draws
open olive groves; other broadleaf stands mix olives and oaks, while conifer
stands mix umbrella pines with cypresses. Limestone outcrops are static geometry
and remain outside the vegetation burn ranges.
