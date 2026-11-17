Models run locally using the system ONNX Runtime. `bin/fetch-models` downloads
these pinned files, verifies SHA-256, and replaces them atomically. The app also
verifies the files before opening the camera. Weights are excluded from Git.

| File | Source | Size |
| --- | --- | --- |
| yunet.onnx | [OpenCV Zoo YuNet](https://github.com/opencv/opencv_zoo/tree/48b10fa505aeb51fbd127fc60a5c660040e823b7/models/face_detection_yunet) | 230 KB |
| landmarks.onnx | [Yin Guobing facial landmarks](https://github.com/yinguobing/head-pose-estimation/tree/e0c37703e31807e49ab46d919cd010a7809b2300) | 29.4 MB |

Upstream MIT notices are retained alongside this file. The landmark project
also notes that its training datasets have their own terms; see its README
before redistributing the weights commercially. No InsightFace detector or
OpenFace 3D face model is used here.

YuNet decoding follows the model's published stride and box parameterization.
The app estimates relative head geometry from 68 landmarks and approximates
eye position from dark pixels inside each eye contour. This is not a trained
gaze estimator or a calibrated angular eye tracker.
