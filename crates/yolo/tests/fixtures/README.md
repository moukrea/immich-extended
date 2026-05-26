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

## `yolo11n.onnx` (≈5 MB, optional)

Ultralytics YOLOv11n nano model, exported to ONNX with `imgsz=640`. AGPL-3.0; we use it
only for test inference, not redistribute. The fixture is bundled so CI does not need
network access. If the file is absent, the integration tests fall back to
`ensure_model` which reads `YOLO_MODEL_URL` + `YOLO_MODEL_SHA256` from the environment.

To re-export:

```bash
python3 -m venv /tmp/yenv && source /tmp/yenv/bin/activate
pip install ultralytics
yolo export model=yolo11n.pt format=onnx imgsz=640
mv yolo11n.onnx crates/yolo/tests/fixtures/
sha256sum crates/yolo/tests/fixtures/yolo11n.onnx  # update creds.env YOLO_MODEL_SHA256
```
