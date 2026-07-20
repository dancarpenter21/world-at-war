import { useState, type ReactNode } from "react";

export type MapFilters = {
  spaceAssets: {
    showAll: boolean;
    showStarlink: boolean;
  };
  runways: {
    visible: boolean;
    minimumLengthM: number;
  };
};

type FilterTabId = "space-assets" | "runways";

export function MapFilterDialog({ filters, spaceAssetsAvailable, onChange, onClose }: {
  filters: MapFilters;
  spaceAssetsAvailable: boolean;
  onChange: (filters: MapFilters) => void;
  onClose: () => void;
}) {
  const [activeTab, setActiveTab] = useState<FilterTabId>("space-assets");
  const tabs: { id: FilterTabId; label: string; content: ReactNode }[] = [
    {
      id: "space-assets",
      label: "Space assets",
      content: <div className="filter-panel">
        <label className="filter-toggle">
          <span><strong>All space assets</strong><small>Display payload assets; use Starlink below to include that constellation.</small></span>
          <input
            aria-label="Show all space assets"
            type="checkbox"
            disabled={!spaceAssetsAvailable}
            checked={filters.spaceAssets.showAll}
            onChange={(event) => onChange({
              ...filters,
              spaceAssets: { ...filters.spaceAssets, showAll: event.target.checked }
            })}
          />
        </label>
        <label className="filter-toggle nested-filter-toggle">
          <span><strong>Starlink</strong><small>Display Starlink payloads on the globe.</small></span>
          <input
            aria-label="Show Starlink"
            type="checkbox"
            disabled={!spaceAssetsAvailable}
            checked={filters.spaceAssets.showStarlink}
            onChange={(event) => onChange({
              ...filters,
              spaceAssets: { ...filters.spaceAssets, showStarlink: event.target.checked }
            })}
          />
        </label>
        {!spaceAssetsAvailable && <p className="muted">The selected scenario does not include an orbital catalog.</p>}
      </div>
    },
    {
      id: "runways",
      label: "Runways",
      content: <div className="filter-panel">
        <label className="filter-toggle">
          <span><strong>Airport runways</strong><small>Display airports that meet the runway length threshold.</small></span>
          <input
            aria-label="Show airport runways"
            type="checkbox"
            checked={filters.runways.visible}
            onChange={(event) => onChange({
              ...filters,
              runways: { ...filters.runways, visible: event.target.checked }
            })}
          />
        </label>
        <label className="runway-length-filter">
          Minimum runway length
          <div><input
            aria-label="Minimum runway length"
            type="number"
            min={0}
            step={100}
            value={filters.runways.minimumLengthM}
            disabled={!filters.runways.visible}
            onChange={(event) => onChange({
              ...filters,
              runways: {
                ...filters.runways,
                minimumLengthM: Math.max(0, Number(event.target.value) || 0)
              }
            })}
          /><span>m</span></div>
          <small>Airports with at least one runway this long are shown.</small>
        </label>
      </div>
    }
  ];

  return <section className="map-filter-dialog" role="dialog" aria-label="Map filters">
    <div className="filter-dialog-header"><div><strong>Map filters</strong><small>Visible layers</small></div><button aria-label="Close map filters" onClick={onClose}>×</button></div>
    <div className="filter-tabs" role="tablist" aria-label="Map filter categories">
      {tabs.map((tab) => <button
        key={tab.id}
        role="tab"
        aria-selected={activeTab === tab.id}
        className={activeTab === tab.id ? "active" : ""}
        onClick={() => setActiveTab(tab.id)}
      >{tab.label}</button>)}
    </div>
    {tabs.find((tab) => tab.id === activeTab)?.content}
  </section>;
}
