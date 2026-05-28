# YOLO crate test fixtures

Public-domain images used by `crates/yolo/tests/inference.rs`. Provenance:

## `one_person.jpg` (33 KB, 400×500 JPEG)

Official White House portrait of President Barack Obama, December 2012.
Photographer: Pete Souza, Executive Office of the President of the United States.

- **Source**: https://commons.wikimedia.org/wiki/File:President_Barack_Obama.jpg
- **License**: Public Domain (work of the U.S. federal government — 17 U.S.C. § 105).
- **Processing**: downscaled from the 2687×3356 original via
  `ffmpeg -i original.jpg -vf scale=400:-1 -q:v 6 one_person.jpg`.

## `empty_landscape.jpg` (37 KB, 400×266 JPEG)

Grand Canyon view from Pima Point, March 2010. Photographer: Murray Foubister.

- **Source**: https://commons.wikimedia.org/wiki/File:Grand_Canyon_view_from_Pima_Point_2010.jpg
- **License**: CC BY-SA 2.0 (we link back here as attribution).
- **Processing**: downscaled from the 2848×4288 original via
  `ffmpeg -i original.jpg -vf scale=400:-1 -q:v 6 empty_landscape.jpg`.

## `10s_one_person.mp4` (40 KB, 640×480 H.264)

10-second synthesised clip looping `one_person.jpg`. Used by
`crates/yolo/tests/video.rs` to exercise the ffmpeg frame extractor and the
end-to-end video person count. Same provenance / licence as `one_person.jpg`
(U.S. federal-government public domain).

Re-synthesis recipe (Fedora 43 ffmpeg ships `libopenh264` but not `libx264`):

```bash
ffmpeg -y -loop 1 -i one_person.jpg -t 10 \
  -vf "scale=640:480,format=yuv420p" -r 25 \
  -c:v libopenh264 -pix_fmt yuv420p 10s_one_person.mp4
```

At `fps=0.5` this clip yields exactly **5 extracted frames** (t = 0, 2, 4, 6, 8 s).

## `yolo11n.onnx` (≈10 MB, optional)

Ultralytics YOLOv11n nano model, exported to ONNX with `imgsz=640`. AGPL-3.0; we
redistribute the exported bytes from a GitHub release of this repo for end-user
convenience only. The fixture is bundled so CI does not need network access. If
the file is absent, the integration tests fall back to `ensure_model`, which
downloads from [`yolo::DEFAULT_MODEL_URL`] (a release asset on this repo). The
`YOLO_MODEL_URL` env var remains as an optional override for advanced operators
who want to point at a different mirror.

To re-export and refresh the bundled bytes:

```bash
python3 -m venv /tmp/yenv && source /tmp/yenv/bin/activate
pip install ultralytics
yolo export model=yolo11n.pt format=onnx imgsz=640
mv yolo11n.onnx crates/yolo/tests/fixtures/
gh release upload models-yolo11n-vN crates/yolo/tests/fixtures/yolo11n.onnx \
  --repo moukrea/immich-extended  # or `create` for a new tag
# then bump DEFAULT_MODEL_URL in crates/yolo/src/model.rs if the tag changed
```
