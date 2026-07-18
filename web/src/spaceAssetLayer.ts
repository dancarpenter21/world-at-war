import { Cartesian3, Color, PointPrimitiveCollection, type PointPrimitive, type Viewer } from "cesium";

type StatusCallback = (status: string) => void;
type SpaceAssetFilters = { showAll: boolean; showStarlink: boolean };

const allAssetFilters = {
  query: "",
  mode: "payloads",
  debris: false,
  rocketBodies: false,
  facets: {}
};

const starlinkFilters = {
  query: "starlink",
  mode: "payloads",
  debris: false,
  rocketBodies: false,
  facets: {}
};

const hiddenFilters = { ...starlinkFilters, query: "__hidden_space_asset_layer__" };

export class SpaceAssetLayer {
  private readonly points: PointPrimitiveCollection;
  private readonly pointsById = new Map<number, PointPrimitive>();
  private readonly worker: Worker;
  private visible = false;
  private ready = false;
  private filters: SpaceAssetFilters = { showAll: false, showStarlink: false };

  constructor(
    private readonly viewer: Viewer,
    gameId: string,
    playerId: string,
    roleId: string,
    private readonly onStatus: StatusCallback
  ) {
    this.points = viewer.scene.primitives.add(new PointPrimitiveCollection());
    this.worker = new Worker(new URL("./spaceAssetWorker.ts", import.meta.url), { type: "module" });
    this.worker.onmessage = (event) => this.handleMessage(event.data);
    this.worker.postMessage({
      type: "init",
      apiBase: import.meta.env.VITE_API_BASE ?? "",
      gameId,
      playerId,
      roleId,
      initialFilters: hiddenFilters
    });
  }

  setFilters(filters: SpaceAssetFilters) {
    this.filters = filters;
    this.visible = filters.showAll || filters.showStarlink;
    for (const point of this.pointsById.values()) point.show = this.visible;
    if (!this.ready) {
      this.onStatus(this.visible ? "Loading space assets" : "Space assets hidden");
      return;
    }
    this.worker.postMessage({ type: "filter", filters: this.workerFilters() });
    if (!this.visible) this.onStatus("Space assets hidden");
  }

  private workerFilters() {
    if (this.filters.showAll) {
      return this.filters.showStarlink ? allAssetFilters : { ...allAssetFilters, excludeQuery: "starlink" };
    }
    return this.filters.showStarlink ? starlinkFilters : hiddenFilters;
  }

  private handleMessage(message: { type: string; [key: string]: unknown }) {
    if (message.type === "error") {
      this.onStatus(String(message.message ?? "Space asset layer unavailable"));
      return;
    }
    if (message.type === "ready") {
      this.ready = true;
      this.worker.postMessage({ type: "filter", filters: this.workerFilters() });
      return;
    }
    if (message.type === "filtered") {
      const ids = message.ids as number[];
      const visibleIds = new Set(ids);
      for (const [id, point] of this.pointsById) point.show = this.visible && visibleIds.has(id);
      if (this.visible) this.onStatus(this.filters.showAll
        ? `${ids.length.toLocaleString()} space assets`
        : `${ids.length.toLocaleString()} Starlink assets`);
      return;
    }
    if (message.type === "positions") {
      const ids = message.ids as Uint32Array;
      const positions = message.positions as Float64Array;
      for (let index = 0; index < ids.length; index++) {
        const id = ids[index];
        let point = this.pointsById.get(id);
        const position = new Cartesian3(positions[index * 3], positions[index * 3 + 1], positions[index * 3 + 2]);
        if (!point) {
          point = this.points.add({
            id: `space-asset:${id}`,
            position,
            pixelSize: 4,
            color: Color.fromCssColorString("#58c9e8"),
            outlineColor: Color.fromCssColorString("#10232b"),
            outlineWidth: 1,
            show: this.visible
          });
          this.pointsById.set(id, point);
        } else {
          point.position = position;
        }
      }
      this.viewer.scene.requestRender();
    }
  }

  destroy() {
    this.worker.terminate();
    this.pointsById.clear();
    if (!this.points.isDestroyed()) this.viewer.scene.primitives.remove(this.points);
  }
}
