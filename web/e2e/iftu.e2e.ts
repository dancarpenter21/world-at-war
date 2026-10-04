import { expect, test } from "@playwright/test";
test("sends authorized update, retarget, handoff and source requests without claiming receipt", async ({page},testInfo)=>{
  const submissions: Record<string, any>[]=[];
  await page.route("**/v1/games/iftu-test/roles/pilot/intent", async route=>{submissions.push(route.request().postDataJSON());await route.fulfill({json:{accepted:true}});});
  await page.goto("/e2e/fixtures/iftu.html");
  await expect(page.getByText("flight unconfirmed · Unconfirmed; no received weapon status")).toBeVisible();
  await page.getByRole("button",{name:"Send target update",exact:true}).click();
  await expect.poll(()=>submissions.length).toBe(1);
  expect(submissions[0].intent.kind.Iftu.command).toEqual({action:"update",track_id:"track-a"});
  expect(submissions[0].intent.target).toBe("launcher");expect(submissions[0].lease_generation).toBe(3);
  await page.getByRole("button",{name:"Retarget weapon",exact:true}).click();
  await expect.poll(()=>submissions.length).toBe(2);expect(submissions[1].intent.kind.Iftu.command.action).toBe("retarget");
  await page.getByLabel("Compatible provider").selectOption("backup");
  await page.getByRole("button",{name:"Request provider handoff",exact:true}).click();
  await expect.poll(()=>submissions.length).toBe(3);expect(submissions[2].intent.kind.Iftu.command).toEqual({action:"assign_provider",provider_id:"backup"});
  await page.getByLabel("Observation source").selectOption("sensor");
  await page.getByRole("button",{name:"Request target reports",exact:true}).click();
  await expect.poll(()=>submissions.length).toBe(4);expect(submissions[3].intent.kind.Iftu.command).toEqual({action:"subscribe",source_id:"sensor",provider_id:"backup"});
  await expect(page.getByRole("status")).toContainText("weapon receipt remains unconfirmed");
  await page.getByLabel("Search weapon messages").fill("track_report");
  await expect(page.locator("details")).toHaveCount(1);
  await page.screenshot({path:testInfo.outputPath("iftu-controls.png"),fullPage:true});
});
test("retries an uncertain submission with its original idempotency key",async({page})=>{
  const ids:string[]=[];
  await page.route("**/v1/games/iftu-test/roles/pilot/intent",async route=>{ids.push(route.request().postDataJSON().intent.intent_id);if(ids.length===1)await route.abort();else await route.fulfill({json:{accepted:true}});});
  await page.goto("/e2e/fixtures/iftu.html");await page.getByRole("button",{name:"Send target update",exact:true}).click();
  await page.getByRole("button",{name:"Retry original command",exact:true}).click();await expect.poll(()=>ids.length).toBe(2);expect(ids[1]).toBe(ids[0]);
});
