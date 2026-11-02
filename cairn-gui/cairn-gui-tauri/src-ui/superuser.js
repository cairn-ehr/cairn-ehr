// The superuser's panel (sign-in slice 2, Task 8). Classic script, loaded after rekey.js (uses
// `signerCredential`, `clearSigner`, and main.js's `el`, `setMessage`, `invoke`, `failureText`).
//
// Two lists, re-read after every write: the members who can be given or lose the enroller role
// (each row also offers Repudiate on that member's current key), and the keys awaiting a
// superuser's judgement. The superuser authenticates through the signer
// strip. Repudiate asks NO question (principle 3, paper-parity): the act is signed, audited, and
// repaired by a re-key, so a dialog would only slow the one person who knows what they are doing.

function superuserButton(label, onPress) {
  const b = document.createElement("button");
  b.type = "button";
  b.textContent = label;
  b.addEventListener("click", () => {
    // In-flight guard: a fast double-click must not send the act twice. The refresh that follows
    // the act redraws the row, which re-enables its buttons (a failed refresh drops the rows).
    const row = b.parentElement;
    row.querySelectorAll("button").forEach((x) => (x.disabled = true));
    onPress();
  });
  return b;
}

function superuserItem(text, buttons) {
  const li = document.createElement("li");
  const span = document.createElement("span");
  span.textContent = text + " ";
  li.append(span, ...buttons);
  return li;
}

/** Run one write with the strip's credential, say the result, and re-read both lists. */
async function superuserAct(command, args) {
  let said;
  try {
    said = await invoke(command, {
      credential: signerCredential() || { staffId: "", passphrase: "" },
      ...args,
    });
  } catch (failure) {
    said = typeof failure === "string" ? failure : failureText(failure);
  }
  // Whatever happened, the superuser's passphrase leaves the screen. `false`: not a clinical act.
  clearSigner(false);
  setMessage(el("superuser-status"), said);
  // A failed refresh appends to the outcome sentence; it must never replace it (an
  // "it may have been made" answer would be lost).
  await superuserRefresh(said);
}

async function superuserRefresh(outcome) {
  const grants = el("superuser-grants");
  const keys = el("superuser-keys");
  try {
    const members = await invoke("list_enrollers");
    grants.replaceChildren(
      ...members.map((m) =>
        superuserItem(m.text, [
          superuserButton(m.enroller ? "Withdraw" : "Grant", () =>
            superuserAct("set_enroller", { staffId: m.staff_id, grant: !m.enroller }),
          ),
          // The member's CURRENT key, carried by the row: a confirmed key (an impostor found
          // later) or an enroller's own is never on the worklist below (§5.5). No question asked.
          ...(m.kid ? [superuserButton("Repudiate key", () =>
            superuserAct("judge_key", { kid: m.kid, judgement: "repudiate" }))] : []),
        ]),
      ),
    );
    const waiting = await invoke("list_unconfirmed");
    keys.replaceChildren(
      ...waiting.map((k) =>
        superuserItem(k.text, [
          superuserButton("Confirm", () => superuserAct("judge_key", { kid: k.kid, judgement: "confirm" })),
          superuserButton("Repudiate", () => superuserAct("judge_key", { kid: k.kid, judgement: "repudiate" })),
        ]),
      ),
    );
    el("superuser-keys-empty").hidden = waiting.length > 0;
  } catch (failure) {
    // Rows that could not be re-read are dropped: a stale row's key must not be pressed.
    grants.replaceChildren();
    keys.replaceChildren();
    const why = typeof failure === "string" ? failure : failureText(failure);
    setMessage(el("superuser-status"), outcome ? outcome + " " + why : why);
  }
}

el("superuser-panel").addEventListener("toggle", (e) => {
  if (e.target.open) superuserRefresh();
});
