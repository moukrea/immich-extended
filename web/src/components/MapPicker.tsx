// Interactive map picker for a circular geo predicate.
//
// Props use [lat, lng] ordering to match Rust `LocationPredicate.center` (PRD §6).
// MapLibre's LngLat API is the opposite — conversions happen at the boundary
// inside this file; callers always speak [lat, lng].
//
// The visual radius is a 64-vertex small-circle polygon computed with the
// haversine destination formula. Pixel-radius `circle` paint was rejected
// because it doesn't track real-world km when the viewport zooms.

import {
  createEffect,
  on,
  onCleanup,
  onMount,
  untrack,
  type Component,
} from "solid-js";
import maplibregl, {
  type GeoJSONSource,
  type Map as MaplibreMap,
  type StyleSpecification,
} from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";

export interface MapPickerProps {
  center: [number, number];
  radiusKm: number;
  onChange: (center: [number, number], radiusKm: number) => void;
}

const EARTH_RADIUS_KM = 6371;
const CIRCLE_STEPS = 64;
const RADIUS_SOURCE_ID = "radius";
const RADIUS_FILL_LAYER_ID = "radius-fill";
const RADIUS_OUTLINE_LAYER_ID = "radius-outline";

const OSM_STYLE: StyleSpecification = {
  version: 8,
  sources: {
    osm: {
      type: "raster",
      tiles: ["https://tile.openstreetmap.org/{z}/{x}/{y}.png"],
      tileSize: 256,
      attribution:
        '© <a href="https://www.openstreetmap.org/copyright" target="_blank" rel="noreferrer noopener">OpenStreetMap contributors</a>',
    },
  },
  layers: [{ id: "osm", type: "raster", source: "osm" }],
};

function circlePolygon(
  centerLatLng: [number, number],
  radiusKm: number,
): GeoJSON.Feature<GeoJSON.Polygon> {
  const [lat, lon] = centerLatLng;
  const latRad = (lat * Math.PI) / 180;
  const lonRad = (lon * Math.PI) / 180;
  const angularDistance = radiusKm / EARTH_RADIUS_KM;
  const ring: GeoJSON.Position[] = [];
  for (let i = 0; i <= CIRCLE_STEPS; i++) {
    const bearing = (i / CIRCLE_STEPS) * 2 * Math.PI;
    const sinLatDest =
      Math.sin(latRad) * Math.cos(angularDistance) +
      Math.cos(latRad) * Math.sin(angularDistance) * Math.cos(bearing);
    const latDest = Math.asin(Math.max(-1, Math.min(1, sinLatDest)));
    const lonDest =
      lonRad +
      Math.atan2(
        Math.sin(bearing) * Math.sin(angularDistance) * Math.cos(latRad),
        Math.cos(angularDistance) - Math.sin(latRad) * Math.sin(latDest),
      );
    ring.push([(lonDest * 180) / Math.PI, (latDest * 180) / Math.PI]);
  }
  return {
    type: "Feature",
    properties: {},
    geometry: { type: "Polygon", coordinates: [ring] },
  };
}

const MapPicker: Component<MapPickerProps> = (props) => {
  let mapDiv!: HTMLDivElement;
  let map: MaplibreMap | undefined;
  let mapLoaded = false;

  const writeRadiusSource = () => {
    if (!map || !mapLoaded) return;
    const source = map.getSource(RADIUS_SOURCE_ID) as GeoJSONSource | undefined;
    if (!source) return;
    source.setData(circlePolygon(props.center, props.radiusKm));
  };

  onMount(() => {
    const instance = new maplibregl.Map({
      container: mapDiv,
      style: OSM_STYLE,
      center: [props.center[1], props.center[0]],
      zoom: 10,
    });
    map = instance;

    instance.on("load", () => {
      mapLoaded = true;
      const initialData = untrack(() =>
        circlePolygon(props.center, props.radiusKm),
      );
      instance.addSource(RADIUS_SOURCE_ID, {
        type: "geojson",
        data: initialData,
      });
      instance.addLayer({
        id: RADIUS_FILL_LAYER_ID,
        type: "fill",
        source: RADIUS_SOURCE_ID,
        paint: {
          "fill-color": "#4f46e5",
          "fill-opacity": 0.15,
        },
      });
      instance.addLayer({
        id: RADIUS_OUTLINE_LAYER_ID,
        type: "line",
        source: RADIUS_SOURCE_ID,
        paint: {
          "line-color": "#4f46e5",
          "line-width": 2,
        },
      });
    });

    instance.on("click", (event) => {
      untrack(() => {
        props.onChange(
          [event.lngLat.lat, event.lngLat.lng],
          props.radiusKm,
        );
      });
    });
  });

  createEffect(
    on(
      () => [props.center[0], props.center[1]] as [number, number],
      ([lat, lng]) => {
        if (!map) return;
        map.setCenter([lng, lat]);
        writeRadiusSource();
      },
      { defer: true },
    ),
  );

  createEffect(
    on(
      () => props.radiusKm,
      () => writeRadiusSource(),
      { defer: true },
    ),
  );

  onCleanup(() => {
    if (map) {
      map.remove();
      map = undefined;
      mapLoaded = false;
    }
  });

  return (
    <div class="space-y-2">
      <div
        ref={mapDiv}
        class="h-72 w-full rounded-md border border-slate-300"
        aria-label="Map picker"
        role="application"
      />
      <div class="flex items-center gap-3">
        <label
          class="text-sm font-medium text-slate-700"
          for="map-picker-radius"
        >
          Radius
        </label>
        <input
          id="map-picker-radius"
          type="range"
          min="0.1"
          max="500"
          step="0.1"
          value={props.radiusKm}
          onInput={(event) =>
            props.onChange(
              props.center,
              Number(event.currentTarget.value),
            )
          }
          class="flex-1"
        />
        <span class="w-24 text-right text-sm tabular-nums text-slate-700">
          {props.radiusKm.toFixed(1)} km
        </span>
      </div>
      <p class="text-xs text-slate-500">
        Click the map to set the center; drag the slider to adjust the radius.
      </p>
    </div>
  );
};

export default MapPicker;
