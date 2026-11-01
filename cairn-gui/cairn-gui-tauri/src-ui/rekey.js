// Forgotten passphrase (sign-in slice 2, Task 6). Classic script, loaded after enrol.js (uses
// `signerCredential`, `clearSigner`, and main.js's `el`, `setMessage`, `invoke`, `failureText`).
//
// The approver (superuser or enroller) authenticates through the signer strip; the member types
// a NEW passphrase twice. The Rust side checks the two match BEFORE asking the vault. Display
// name, phone and email are sent blank: a blank keeps what the vault holds.

async function rekeyStaff(event) {
  event.preventDefault();
  const status = el("rekey-status");
  let said;
  let rekeyed = false;
  try {
    said = await invoke("rekey_member", {
      credential: signerCredential() || { staffId: "", passphrase: "" },
      member: {
        staffId: el("rekey-staff-id").value,
        role: el("rekey-role").value,
        displayName: "",
        phone: "",
        email: "",
        passphrase: el("rekey-passphrase").value,
        passphraseAgain: el("rekey-passphrase-again").value,
        hint: el("rekey-hint").value,
      },
    });
    rekeyed = true;
  } catch (failure) {
    said = typeof failure === "string" ? failure : failureText(failure);
  }
  // Both ways out: no new passphrase stays on a shared screen, nor the approver's.
  el("rekey-passphrase").value = "";
  el("rekey-passphrase-again").value = "";
  clearSigner(false);
  if (rekeyed) {
    el("rekey-staff-id").value = "";
    el("rekey-hint").value = "";
  }
  setMessage(status, said);
}

el("rekey-form").addEventListener("submit", rekeyStaff);
