import { type Component } from "solid-js";
import type { FaceRecognitionLeaf } from "../../lib/matchTree";
import BlockShell from "./BlockShell";

interface Props {
  leaf: FaceRecognitionLeaf;
  onChange: (next: FaceRecognitionLeaf) => void;
  onRemove: () => void;
}

const FaceRecognitionBlock: Component<Props> = (props) => {
  const onAllowChange = (allow_unrecognized: boolean) =>
    props.onChange({ ...props.leaf, allow_unrecognized });
  const onYoloChange = (yolo_count_check: boolean) =>
    props.onChange({ ...props.leaf, yolo_count_check });
  return (
    <BlockShell
      title="Face recognition"
      badge={props.leaf.yolo_count_check ? "YOLO" : undefined}
      testid="block-face-recognition"
      onRemove={props.onRemove}
    >
      <label class="flex items-start gap-2 text-sm text-immich-fg dark:text-immich-dark-fg">
        <input
          type="checkbox"
          class="mt-0.5"
          checked={props.leaf.allow_unrecognized}
          onChange={(e) => onAllowChange(e.currentTarget.checked)}
          aria-label="Allow unrecognized faces"
        />
        <span>
          Allow unrecognized faces
          <span class="block text-xs text-ui-muted dark:text-gray-400">
            When OFF, an Immich-detected face that isn&apos;t in the roster of
            included people blocks the match.
          </span>
        </span>
      </label>
      <label class="flex items-start gap-2 text-sm text-immich-fg dark:text-immich-dark-fg">
        <input
          type="checkbox"
          class="mt-0.5"
          checked={props.leaf.yolo_count_check}
          onChange={(e) => onYoloChange(e.currentTarget.checked)}
          aria-label="YOLO count check"
        />
        <span>
          YOLO count check
          <span class="block text-xs text-ui-muted dark:text-gray-400">
            Skip when the on-prem human detector counts more people than Immich
            identified faces.
          </span>
        </span>
      </label>
    </BlockShell>
  );
};

export default FaceRecognitionBlock;
