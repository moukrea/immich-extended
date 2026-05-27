import { type Component } from "solid-js";
import type { MediaTypeLeaf, MediaTypeValue } from "../../lib/matchTree";
import BlockShell from "./BlockShell";

interface Props {
  leaf: MediaTypeLeaf;
  onChange: (next: MediaTypeLeaf) => void;
  onRemove: () => void;
}

function toggle(types: MediaTypeValue[], type: MediaTypeValue, on: boolean): MediaTypeValue[] {
  const set = new Set(types);
  if (on) set.add(type);
  else set.delete(type);
  // Preserve a stable photo-before-video order so serialization stays canonical.
  const out: MediaTypeValue[] = [];
  if (set.has("photo")) out.push("photo");
  if (set.has("video")) out.push("video");
  return out;
}

const MediaTypeBlock: Component<Props> = (props) => {
  const onTogglePhoto = (on: boolean) =>
    props.onChange({ ...props.leaf, types: toggle(props.leaf.types, "photo", on) });
  const onToggleVideo = (on: boolean) =>
    props.onChange({ ...props.leaf, types: toggle(props.leaf.types, "video", on) });
  return (
    <BlockShell
      title="Media type"
      testid="block-media-type"
      onRemove={props.onRemove}
    >
      <div class="flex gap-4">
        <label class="inline-flex items-center gap-2 text-sm text-immich-fg dark:text-immich-dark-fg">
          <input
            type="checkbox"
            checked={props.leaf.types.includes("photo")}
            onChange={(e) => onTogglePhoto(e.currentTarget.checked)}
            aria-label="Photo"
          />
          Photo
        </label>
        <label class="inline-flex items-center gap-2 text-sm text-immich-fg dark:text-immich-dark-fg">
          <input
            type="checkbox"
            checked={props.leaf.types.includes("video")}
            onChange={(e) => onToggleVideo(e.currentTarget.checked)}
            aria-label="Video"
          />
          Video
        </label>
      </div>
    </BlockShell>
  );
};

export default MediaTypeBlock;
