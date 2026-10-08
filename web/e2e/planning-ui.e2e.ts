import { expect, test } from "@playwright/test";
import { readFile } from "node:fs/promises";

// Browser contract test with explicit API fixtures; backend integration is tested separately.
test("commander compares, edits and publishes a campaign through the planning workspace", async ({ page }) => {
  const plan = JSON.parse(await readFile(new URL("../../data/scenarios/regional-campaign.json", import.meta.url), "utf8"));
  const commander = { id: plan.commander_role_id, name: "Joint Force Commander", side: "Blue", kind: "joint_force_commander", location_unit_id: "hq", command_units: [], held: false, held_by_you: true, ai_controlled: false, lease_generation: 1 };
  const game = { id: "campaign", title: "Regional Joint Campaign", status: "lobby", host_player_id: "", player_roles_available: 1, space_catalog_enabled: false };
  let published = false;
  const actions: string[] = [];
  await page.route("**/v1/**", async route => {
    const request = route.request(); const url = new URL(request.url()); const path = url.pathname;
    const headers = { "access-control-allow-origin": new URL(page.url()).origin, "access-control-allow-credentials": "true", "access-control-allow-headers": "content-type, x-csrf-token", "access-control-allow-methods": "GET, POST, OPTIONS" };
    if (request.method() === "OPTIONS") { await route.fulfill({ status:204,headers }); return; }
    let body: unknown = {};
    if (path.startsWith("/v1/auth/")) body = { player_id: "fixture-player", display_name: "Commander", csrf_token: "fixture-csrf", expires_unix: 9999999999 };
    else if (path.endsWith("/renew") || path.endsWith("/resume")) body = commander;
    else if (path === "/v1/scenarios") body = [{ id:"regional-campaign.v1",title:game.title,description:"Offline campaign",version:1,authored_entity_count:12,role_count:10,requires_space_catalog:false }];
    else if (path.includes("space-catalog/status")) body = { configured:false,usable:false,remembered_credentials:false };
    else if (path === "/v1/games") {
      if (request.method() === "POST") { game.host_player_id="fixture-player"; body={game}; } else body=game.host_player_id ? [game] : [];
    } else if (path.endsWith("/roles")) body=[commander];
    else if (path.endsWith("/claim")) { commander.held=true; body=commander; }
    else if (path.endsWith("/start")) { game.status="running"; body=game; }
    else if (path.endsWith("/authority")) body={version:1,roles:[],relationships:[],policies:[]};
    else if (path.endsWith("/authority/requests")) body=[];
    else if (path.endsWith("/state")) body={tick:10,own_units:[],tracks:[],jamming_regions:[],communication_links:[]};
    else if (path.endsWith("/planning")) {
      if (request.method()==="POST") {
        const action=request.postDataJSON(); actions.push(action.action);
        if(action.action==="save") Object.assign(plan,action.plan,{revision:plan.revision+1});
        if(action.action==="publish") { plan.selected_course_id=action.course_id; plan.published_tick=10; published=true; }
      }
      body={draft:plan,received:published?plan:null,comparisons:[],reports:[],clearances:[],handoffs:[],proposals:[],tick:10};
    } else if (path==="/v1/airports") body={airports:[],total:0};
    await route.fulfill({ json:body,headers });
  });
  await page.goto("/");
  await page.getByRole("button",{name:"Create game",exact:true}).click();
  await page.getByRole("button",{name:/Joint Force Commander/}).click();
  await page.getByRole("button",{name:"Start scenario",exact:true}).click();
  await page.getByRole("button",{name:"Joint planning",exact:true}).click();
  await expect(page.getByRole("heading",{name:"Compare courses of action"})).toBeVisible();
  await page.getByLabel("Commander intent",{exact:true}).fill("Protect the joint force and preserve safe transit.");
  await page.getByRole("button",{name:"Save draft",exact:true}).click();
  await expect(page.getByRole("region", { name: "Joint campaign planning" }).getByRole("status")).toContainText("Draft saved");
  await page.getByRole("button",{name:"Approve and publish selected course",exact:true}).click();
  await expect(page.getByText("Received revision 2",{exact:false})).toBeVisible();
  expect(actions).toEqual(["save","publish"]);
  await page.locator(".planning-workspace").evaluate(element => { if ("scrollTop" in element) element.scrollTop = 0; });
  await page.screenshot({path:"test-results/joint-planning.png",fullPage:true});
});
