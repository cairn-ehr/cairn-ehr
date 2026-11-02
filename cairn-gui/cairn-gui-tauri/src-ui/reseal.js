// Change my passphrase (sign-in slice 2, Task 7). Classic script, loaded after rekey.js (uses
// main.js's `el`, `setMessage`, `invoke`, `failureText`). The Rust side checks the new passphrase
// differs from the current one and matches its repeat BEFORE asking the vault.

async function changePassphrase(event) {
  event.preventDefault();
  const status = el("reseal-status");
  let said;
  let changed = false;
  try {
    said = await invoke("change_passphrase", {
      form: {
        staffId: el("reseal-staff-id").value,
        current: el("reseal-current").value,
        new: el("reseal-new").value,
        newAgain: el("reseal-new-again").value,
        hint: el("reseal-hint").value,
      },
    });
    changed = true;
  } catch (failure) {
    said = typeof failure === "string" ? failure : failureText(failure);
  }
  // Both ways out: no passphrase stays on a shared screen.
  el("reseal-current").value = "";
  el("reseal-new").value = "";
  el("reseal-new-again").value = "";
  if (changed) el("reseal-hint").value = "";
  setMessage(status, said);
}

el("reseal-form").addEventListener("submit", changePassphrase);
