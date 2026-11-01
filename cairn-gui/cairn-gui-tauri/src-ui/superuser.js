// The superuser's panel (sign-in slice 2, Task 8). Classic script, loaded after rekey.js (uses
// `signerCredential`, `clearSigner`, and main.js's `el`, `setMessage`, `invoke`, `failureText`).
//
// Two lists, re-read after every write: the members who can be given or lose the enroller role,
// and the keys awaiting a superuser's judgement. The superuser authenticates through the signer
// strip. Repudiate asks NO question (principle 3, paper-parity): the act is signed, audited, and
// repaired by a re-key, so a dialog would only slow the one person who knows what they are doing.

function superuserButton(label, onPress) {
  const b = document.createElement("button");
  b.type = "button";
  b.textContent = label;
  b.addEventListener("click", onPress);
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
  await superuserRefresh();
}

async function superuserRefresh() {
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
        ]),
      ),
    );
    const waiting = await invoke("list_unconfirmed");
    keys.replaceChildren(
      ...waiting.map((k) =>
        superuserItem(k.text, [
          superuserButton("Confirm", () => superuserAct("judge_key", { kid: k.kid, confirm: true })),
          superuserButton("Repudiate", () => superuserAct("judge_key", { kid: k.kid, confirm: false })),
        ]),
      ),
    );
    el("superuser-keys-empty").hidden = waiting.length > 0;
  } catch (failure) {
    setMessage(el("superuser-status"), typeof failure === "string" ? failure : failureText(failure));
  }
}

el("superuser-panel").addEventListener("toggle", (e) => {
  if (e.target.open) superuserRefresh();
});
