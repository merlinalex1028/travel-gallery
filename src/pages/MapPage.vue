<script setup lang="ts">
import { locale, tr } from '../i18n';

import CustomSelect from '../components/CustomSelect.vue';
import { computed, onMounted, onUnmounted, shallowRef, useTemplateRef, watch } from 'vue';
import { useRouter } from 'vue-router';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { api } from '../services/api';
import type { Commands, MapPoint } from '../services/contracts';
import type { GeoJSONSourceSpecification } from 'maplibre-gl';
import Icon from '../components/Icon.vue';
import EmptyState from '../components/EmptyState.vue';
import { loadOfflineMapTile, offlineMapProtocol, offlineWorldSource } from '../services/offline-map';
const app = useApp(),
  lib = useLibrary(),
  router = useRouter(),
  element = useTemplateRef<HTMLElement>('map-root'),
  points = shallowRef<MapPoint[]>([]),
  error = shallowRef(''),
  mapError = shallowRef(''),
  ready = shallowRef(false),
  loading = shallowRef(true);
const tripId = shallowRef(''),
  showTrails = shallowRef(false),
  trails = shallowRef<Commands['map_trails']['result']>({
    type: 'FeatureCollection',
    features: [],
    totalPhotos: 0,
    includedPhotos: 0,
    truncated: false,
    unknownTimezone: 0,
    skippedInvalidTimes: 0,
  });
const locationPage = shallowRef(1),
  locationPages = computed(() => Math.max(1, Math.ceil(points.value.length / 100))),
  visibleLocations = computed(() =>
    points.value.slice((locationPage.value - 1) * 100, locationPage.value * 100),
  );
let map: import('maplibre-gl').Map | undefined;
let destroyed = false,
  generation = 0,
  pendingSourceRefresh = false,
  loadTimer: ReturnType<typeof setTimeout> | undefined;
const markers = new Map<string, import('maplibre-gl').Marker>();
function mapPalette() {
  const style = getComputedStyle(document.documentElement);
  return Object.fromEntries(
    ['ocean', 'land', 'coast', 'trail'].map((name) => [name, style.getPropertyValue(`--map-${name}`).trim()]),
  );
}
function updateMapPalette() {
  if (!map || !ready.value) return;
  const colors = mapPalette();
  map.setPaintProperty('ocean', 'background-color', colors.ocean);
  map.setPaintProperty('land', 'fill-color', colors.land);
  map.setPaintProperty('coast', 'line-color', colors.coast);
  map.setPaintProperty('trails', 'line-color', colors.trail);
}
watch(() => app.isDark, updateMapPalette, { flush: 'post' });
function open(point: MapPoint) {
  lib.setFilter({ gpsCell: [point.lat, point.lon], ...(tripId.value ? { collectionId: tripId.value } : {}) });
  void router.push('/filter?showResults=1');
}
function fitLocations(locations: MapPoint[]) {
  if (!map || !locations.length) return;
  if (locations.length === 1) {
    map.jumpTo({ center: [locations[0]!.lon, locations[0]!.lat], zoom: 6 });
    return;
  }
  let west = 180,
    east = -180,
    south = 85,
    north = -85;
  for (const point of locations) {
    west = Math.min(west, point.lon);
    east = Math.max(east, point.lon);
    const latitude = Math.max(-85, Math.min(85, point.lat));
    south = Math.min(south, latitude);
    north = Math.max(north, latitude);
  }
  map.fitBounds(
    [
      [west, south],
      [east, north],
    ],
    { padding: 60, maxZoom: 7, animate: false },
  );
}
function photoData(): Exclude<GeoJSONSourceSpecification['data'], string | undefined> {
  return {
    type: 'FeatureCollection',
    features: points.value.map((point, id) => ({
      type: 'Feature',
      geometry: { type: 'Point', coordinates: [point.lon, point.lat] },
      properties: { ...point, id },
    })),
  };
}
async function reload() {
  const seq = ++generation;
  loading.value = true;
  try {
    const filter = tripId.value ? { collectionId: tripId.value } : {};
    const [locations, lines] = await Promise.all([
      api('map_assets', { filter }),
      api('map_trails', { filter }),
    ]);
    if (destroyed || seq !== generation) return;
    points.value = locations;
    trails.value = lines;
    locationPage.value = 1;
    error.value = '';
    for (const marker of markers.values()) marker.remove();
    markers.clear();
    if (map && ready.value) {
      await Promise.all([
        (map.getSource('photos') as import('maplibre-gl').GeoJSONSource).setData(photoData(), true),
        (map.getSource('trails') as import('maplibre-gl').GeoJSONSource).setData(trails.value, true),
      ]);
      if (!destroyed && seq === generation) fitLocations(locations);
    } else if (map) pendingSourceRefresh = true;
  } catch {
    if (!destroyed && seq === generation) error.value = tr('无法更新地图范围，请重试。');
  } finally {
    if (!destroyed && seq === generation) loading.value = false;
  }
}
watch(showTrails, (show) => {
  if (map?.getLayer('trails')) map.setLayoutProperty('trails', 'visibility', show ? 'visible' : 'none');
});
watch(locale, () => {
  for (const marker of markers.values()) marker.remove();
  markers.clear();
  map?.triggerRepaint();
});
onMounted(async () => {
  try {
    const [maplibre] = await Promise.all([import('maplibre-gl'), reload()]);
    if (!element.value || destroyed) return;
    maplibre.addProtocol(offlineMapProtocol, loadOfflineMapTile);
    const colors = mapPalette();
    map = new maplibre.Map({
      container: element.value,
      attributionControl: false,
      center: [100, 30],
      zoom: 1.8,
      maxZoom: 14,
      style: {
        version: 8,
        sources: {
          world: offlineWorldSource,
          trails: { type: 'geojson', data: trails.value },
          photos: {
            type: 'geojson',
            cluster: true,
            clusterRadius: 50,
            clusterMaxZoom: 13,
            clusterProperties: { photo_count: ['+', ['get', 'count']] },
            data: photoData(),
          },
        },
        layers: [
          { id: 'ocean', type: 'background', paint: { 'background-color': colors.ocean } },
          {
            id: 'land',
            type: 'fill',
            source: 'world',
            'source-layer': 'land',
            paint: { 'fill-color': colors.land },
          },
          {
            id: 'coast',
            type: 'line',
            source: 'world',
            'source-layer': 'land',
            paint: { 'line-color': colors.coast, 'line-width': 1 },
          },
          {
            id: 'trails',
            type: 'line',
            source: 'trails',
            layout: { visibility: 'none', 'line-join': 'round', 'line-cap': 'round' },
            paint: {
              'line-color': colors.trail,
              'line-width': 2,
              'line-opacity': 0.7,
              'line-dasharray': [2, 2],
            },
          },
          {
            id: 'photo-source',
            type: 'circle',
            source: 'photos',
            paint: { 'circle-radius': 0, 'circle-opacity': 0 },
          },
        ],
      },
    });
    map.addControl(new maplibre.NavigationControl({ showCompass: false }), 'top-right');
    loadTimer = setTimeout(() => {
      if (!ready.value && !destroyed)
        mapError.value = tr('离线地图没有完成加载，可通过下方坐标列表查看照片。');
    }, 10000);
    function updateMarkers() {
      if (!map || !ready.value) return;
      const visible = new Set<string>();
      for (const feature of map.querySourceFeatures('photos')) {
        if (feature.geometry.type !== 'Point') continue;
        const p = feature.properties;
        const key = p.cluster ? `cluster-${p.cluster_id}` : `point-${p.id}`;
        visible.add(key);
        if (markers.has(key)) continue;
        const [lon, lat] = feature.geometry.coordinates;
        if (lon === undefined || lat === undefined) continue;
        const count = Number(p.cluster ? p.photo_count : p.count);
        const button = document.createElement('button');
        button.className = 'map-marker';
        button.textContent = String(count);
        button.setAttribute(
          'aria-label',
          p.cluster
            ? tr('放大查看 {0} 张照片的位置', { 0: count })
            : tr('查看此处的 {0} 张照片', { 0: count }),
        );
        button.addEventListener('click', () => {
          if (!p.cluster) {
            // Tile geometry is quantized; use the original aggregate cell.
            open({ lon: Number(p.lon), lat: Number(p.lat), count, assetId: String(p.assetId) });
            return;
          }
          void app.perform(async () => {
            const current = map;
            const source = current?.getSource('photos') as import('maplibre-gl').GeoJSONSource | undefined;
            if (!source) return;
            const zoom = await source.getClusterExpansionZoom(Number(p.cluster_id));
            if (map === current) current?.easeTo({ center: [lon, lat], zoom: Math.min(14, zoom + 0.3) });
          });
        });
        markers.set(key, new maplibre.Marker({ element: button }).setLngLat([lon, lat]).addTo(map));
      }
      for (const [key, marker] of markers)
        if (!visible.has(key)) {
          marker.remove();
          markers.delete(key);
        }
    }
    map.on('idle', updateMarkers);
    fitLocations(points.value);
    map.on('load', () => {
      if (loadTimer) clearTimeout(loadTimer);
      mapError.value = '';
      ready.value = true;
      updateMapPalette();
      if (pendingSourceRefresh) {
        pendingSourceRefresh = false;
        void reload();
      }
    });
    map.on('error', () => {
      mapError.value = tr('地图资源暂时无法加载，可通过下方坐标列表查看照片。');
    });
  } catch {
    error.value = points.value.length
      ? tr('地图暂时不可用，可通过下方地点列表查看照片。')
      : tr('暂时无法加载地图或照片位置，请重新进入此页重试。');
  } finally {
    loading.value = false;
  }
});
onUnmounted(() => {
  destroyed = true;
  if (loadTimer) clearTimeout(loadTimer);
  generation++;
  for (const marker of markers.values()) marker.remove();
  markers.clear();
  map?.remove();
  map = undefined;
});
</script>
<template>
  <section class="standard-page map-page">
    <div class="page-heading">
      <div>
        <div class="eyebrow">THE PLACES WE CARRY WITH US</div>
        <h1>{{ tr('足迹地图') }}</h1>
        <p>{{ tr('世界很大，去过的地方都在心里。') }}</p>
      </div>
      <span class="state-pill online"><Icon name="offline" :size="14" />{{ tr('完全离线') }}</span>
    </div>
    <div class="notice">
      <Icon name="info" />{{
        tr(
          '离线世界概览采用 Natural Earth 公共领域数据，供定位参考，不包含街道。只展示媒体中的真实 GPS；点击位置可查看此处全部照片，不向网络发送坐标。',
        )
      }}
    </div>
    <div class="row map-options">
      <label
        >{{ tr('旅行范围') }}
        <CustomSelect
          v-model="tripId"
          :disabled="loading"
          :aria-label="tr('地图旅行范围')"
          @change="reload"
          :options="[
            { value: '', label: tr('全部媒体') },
            ...app.trips.map((trip) => ({ value: trip.id, label: trip.name })),
          ]"
      /></label>
      <label class="check-label"
        ><input v-model="showTrails" type="checkbox" :disabled="!ready || !trails.features.length" />{{
          tr('显示按日拍摄顺序提示')
        }}</label
      >
    </div>
    <p v-if="showTrails" class="muted small">
      {{
        tr(
          '连线仅表示同日照片的拍摄顺序，不代表实际行走路线。跨日、时区变化、间隔至少6小时或跨越180°经线时断开。',
        )
      }}{{ trails.unknownTimezone ? tr('部分照片时区未知，顺序按媒体墙上时间排列。') : ''
      }}{{ trails.truncated ? tr('仅显示时间最早的5000张定位照片，请按旅行缩小范围。') : '' }}
    </p>
    <div ref="map-root" class="offline-map" :aria-label="tr('离线照片足迹地图')" />
    <p v-if="error || mapError" class="warning-text" role="alert">{{ error || mapError }}</p>
    <div v-if="points.length" class="map-locations">
      <button v-for="point in visibleLocations" :key="`${point.lon}-${point.lat}`" @click="open(point)">
        <Icon name="locate" /><span
          ><strong>{{ point.count }} {{ tr('张照片') }}</strong
          ><small
            >{{ Math.abs(point.lat).toFixed(2) }}° {{ point.lat < 0 ? 'S' : 'N' }},
            {{ Math.abs(point.lon).toFixed(2) }}° {{ point.lon < 0 ? 'W' : 'E' }}</small
          ></span
        ><Icon name="right" :size="16" />
      </button>
    </div>
    <nav v-if="locationPages > 1" class="row" :aria-label="tr('地点列表分页')">
      <span
        >{{ points.length.toLocaleString(locale) }} {{ tr('个位置 · 第') }} {{ locationPage }} /
        {{ locationPages }} {{ tr('页') }}</span
      >
      <button class="btn" :disabled="locationPage === 1" @click="locationPage--">{{ tr('上一页') }}</button>
      <button class="btn" :disabled="locationPage === locationPages" @click="locationPage++">
        {{ tr('下一页') }}
      </button>
    </nav>
    <EmptyState
      v-if="!points.length && !error"
      :icon="loading ? 'loader' : 'map'"
      :title="loading ? tr('正在准备离线地图…') : tr('还没有带 GPS 的旅行照片')"
      :description="
        loading ? tr('正在读取本地位置和地图资源。') : tr('未记录位置的照片不会被推断出精确地点。')
      "
    />
  </section>
</template>
