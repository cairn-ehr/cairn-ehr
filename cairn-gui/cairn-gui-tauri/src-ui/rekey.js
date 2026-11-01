// Forgotten passphrase (sign-in slice 2, Task 6). Classic script, loaded after enrol.js (uses
// `signerCredential`, `clearSigner`, and main.js's `el`, `setMessage`, `invoke`, `failureText`).
//
// The approver (superuser or enroller) authenticates through the signer strip; the member types
// a NEW passphrase twice. The Rust side checks the two match BEFORE asking the vault. Display
// name, phone and email are sent as typed: a blank keeps what the vault holds, and after a restore
// (no vault row, design §5.8) the Rust side asks for a name and a contact before any key is released.

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
        displayName: el("rekey-name").value,
        phone: el("rekey-phone").value,
        email: el("rekey-email").value,
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
    for (const id of ["rekey-staff-id", "rekey-name", "rekey-phone", "rekey-email", "rekey-hint"]) {
      el(id).value = "";
    }
  }
  setMessage(status, said);
}

el("rekey-form").addEventListener("submit", rekeyStaff);
