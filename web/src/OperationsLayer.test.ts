import { EntityCollection } from "cesium";
import { describe, expect, it } from "vitest";
import { OperationsLayer } from "./OperationsLayer";
import type { CampaignPlan, PlanningView } from "./PlanningWorkspace";

const plan: CampaignPlan = { id: "plan", revision: 1, name: "Campaign", intent: "Protect", commander_role_id: "jfc", airspace_authority_role_id: "aca", objectives: [], phases: [], courses: [], selected_course_id: null, published_tick: 0,
  airspaces: [{ id:"west",name:"West",kind:"sector",polygon:[{latitude_deg:38,longitude_deg:-77,altitude_m:0},{latitude_deg:38,longitude_deg:-76,altitude_m:0},{latitude_deg:39,longitude_deg:-76,altitude_m:0}],floor_m:0,ceiling_m:10000,start_tick:10,end_tick:20,controller_role_id:"controller",control_method:"procedural" }] };
const view = (tick: number, received: CampaignPlan | null): PlanningView => ({ tick, received, draft: plan, comparisons: [], reports: [], handoffs: [], clearances: [] });

describe("delivered operational overlays", () => {
  it("does not render unpublished drafts or inactive volumes", () => {
    const entities = new EntityCollection(); const layer = new OperationsLayer(entities);
    layer.update(view(12,null)); expect(entities.values).toHaveLength(0);
    layer.update(view(9,plan)); expect(entities.values).toHaveLength(0);
    layer.update(view(10,plan)); expect(entities.values).toHaveLength(1);
    const entity = entities.values[0]; layer.update(view(11,plan)); expect(entities.values[0]).toBe(entity);
    layer.update(view(20,plan)); expect(entities.values).toHaveLength(0);
  });
  it("removes obsolete airspace geometry when a new revision is delivered", () => {
    const entities = new EntityCollection(); const layer = new OperationsLayer(entities);
    layer.update(view(12,plan)); layer.update(view(12,{...plan,revision:2,airspaces:[]}));
    expect(entities.values).toHaveLength(0);
  });
  it("restores the same received plan after the layer is cleared", () => {
    const entities = new EntityCollection(); const layer = new OperationsLayer(entities);
    layer.update(view(12, plan)); layer.clear(); layer.update(view(12, plan));
    expect(entities.values).toHaveLength(1);
  });
});
