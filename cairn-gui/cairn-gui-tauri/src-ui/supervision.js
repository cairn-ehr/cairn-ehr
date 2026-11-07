// The supervision panels (sign-in slice 3c-3a, Task 8). Classic script, loaded after superuser.js
// (uses its `superuserButton` and `superuserItem`), signer.js's `signerCredential` and
// `clearSigner`, funnel.js's `failureText` (called only at run time), and main.js's `el`,
// `setMessage`, `invoke`.
//
// Two panels over one command, `supervision_act`: the assigner's panel (every member, all six
// acts) and "My supervisees" (the members the person named in who's here supervises: lift their
// co-signing, or step away). The buttons drawn follow each row's state; whether the act is allowed
// is the node's decision alone, said back in one sentence. No act asks a question (principle 3):
// each is signed, audited, and undone by its opposite act.

/** A row button whose accessible name says whose row it acts on. */
function supervisionButton(label, accessibleName, onPress) {
  const b = superuserButton(label, onPress);
  b.setAttribute("aria-label", accessibleName);
  return b;
}

/** Run one act with the strip's credential, say the result, and re-read the pressing panel. */
async function supervisionAct(refresh, statusId, op, staffId, supervisor) {
  let said;
  try {
    said = await invoke("supervision_act", {
      credential: signerCredential() || { staffId: "", passphrase: "" },
      op,
      staffId,
      supervisor: supervisor || null,
    });
  } catch (failure) {
    said = typeof failure === "string" ? failure : failureText(failure);
  }
  // Whatever happened, the passphrase leaves the screen. `false`: not a clinical act.
  clearSigner(false);
  setMessage(el(statusId), said);
  // A failed refresh appends to the outcome sentence; it must never replace it.
  await refresh(said);
}

function assignerAct(op, staffId, supervisor) {
  return supervisionAct(supervisionRefresh, "supervision-status", op, staffId, supervisor);
}

/** One member's row in the assigner's panel: the buttons its state allows asking for. */
function assignerRow(m) {
  const who = m.display_name;
  const controls = [];
  // A student needs co-signing by role: there is nothing to require or lift.
  if (!m.student) {
    const label = m.needs_cosigning ? "Lift co-signing" : "Require co-signing";
    const op = m.needs_cosigning ? "lift-cosign" : "require-cosign";
    controls.push(supervisionButton(label, label + " for " + who, () => assignerAct(op, m.staff_id)));
  }
  const right = m.assigner ? "Withdraw assign-supervision right" : "Grant assign-supervision right";
  const rightOp = m.assigner ? "withdraw-assigner" : "grant-assigner";
  controls.push(supervisionButton(right, right + " for " + who, () => assignerAct(rightOp, m.staff_id)));
  for (const s of m.supervisors) {
    controls.push(supervisionButton("Unlink " + s, "Unlink " + s + " from " + who, () =>
      assignerAct("unlink-supervisor", m.staff_id, s)));
  }
  const input = document.createElement("input");
  input.type = "text";
  input.autocomplete = "off";
  input.setAttribute("aria-label", "Supervisor's staff ID for " + who);
  controls.push(input, supervisionButton("Link a supervisor", "Link a supervisor for " + who, () =>
    assignerAct("link-supervisor", m.staff_id, input.value)));
  return superuserItem(m.text, controls);
}

async function supervisionRefresh(outcome) {
  const list = el("supervision-staff");
  try {
    const members = await invoke("list_supervision");
    list.replaceChildren(...members.map(assignerRow));
  } catch (failure) {
    // Rows that could not be re-read are dropped: a stale row must not be pressed.
    list.replaceChildren();
    const why = typeof failure === "string" ? failure : failureText(failure);
    setMessage(el("supervision-status"), outcome ? outcome + " " + why : why);
  }
}

function superviseeAct(op, staffId, supervisor) {
  return supervisionAct(superviseesRefresh, "my-supervisees-status", op, staffId, supervisor);
}

/** One supervisee's row: lift their co-signing (when it is required, not by role), or step away. */
function superviseeRow(m) {
  const who = m.display_name;
  const controls = [];
  if (m.needs_cosigning && !m.student) {
    controls.push(supervisionButton("Lift co-signing", "Lift co-signing for " + who, () =>
      superviseeAct("lift-cosign", m.staff_id)));
  }
  // The row's own spelling of me (the list narrows `supervisors` to it).
  controls.push(supervisionButton("Step away", "Step away from supervising " + who, () =>
    superviseeAct("unlink-supervisor", m.staff_id, m.supervisors[0])));
  return superuserItem(m.text, controls);
}

async function superviseesRefresh(outcome) {
  const list = el("my-supervisees-list");
  const empty = el("my-supervisees-empty");
  try {
    const mine = await invoke("list_my_supervisees");
    list.replaceChildren(...mine.map(superviseeRow));
    empty.hidden = mine.length > 0;
  } catch (failure) {
    // Dropped, as above; nobody named in who's here also lands here, with its own sentence.
    list.replaceChildren();
    empty.hidden = true;
    const why = typeof failure === "string" ? failure : failureText(failure);
    setMessage(el("my-supervisees-status"), outcome ? outcome + " " + why : why);
  }
}

el("supervision-panel").addEventListener("toggle", (e) => {
  if (e.target.open) supervisionRefresh();
});
el("my-supervisees").addEventListener("toggle", (e) => {
  if (e.target.open) superviseesRefresh();
});
