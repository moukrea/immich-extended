import { lazy, Suspense, type Component } from "solid-js";
import type { LocationLeaf } from "../../lib/matchTree";
import BlockShell from "./BlockShell";

const MapPicker = lazy(() => import("../MapPicker"));

interface Props {
  leaf: LocationLeaf;
  onChange: (next: LocationLeaf) => void;
  onRemove: () => void;
}

const LocationBlock: Component<Props> = (props) => {
  const onMapChange = (center: [number, number], radius_km: number) =>
    props.onChange({ ...props.leaf, center, radius_km });
  return (
    <BlockShell
      title="Location"
      testid="block-location"
      onRemove={props.onRemove}
    >
      <p class="text-xs text-ui-muted dark:text-gray-400">
        Click the map to set the center; the slider sets the radius.
      </p>
      <div data-testid="block-location-map">
        <Suspense
          fallback={<p class="text-sm text-ui-muted">Loading map…</p>}
        >
          <MapPicker
            center={props.leaf.center}
            radiusKm={props.leaf.radius_km}
            onChange={onMapChange}
          />
        </Suspense>
      </div>
    </BlockShell>
  );
};

export default LocationBlock;
