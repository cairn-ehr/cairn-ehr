// Enrol staff (sign-in slice 1, Task 7). Classic script, loaded after signer.js (uses its
// `signerCredential`, `clearSigner`, and main.js's `el`, `setMessage`, `invoke`).
//
// The superuser authenticates through the signer strip; the new member types their own
// passphrase twice into this form. The Rust side checks the two match BEFORE asking the vault.

/** The form as the `member` argument. Passphrases are sent as typed, never trimmed. */
function enrolMember() {
  return {
    staffId: el("enrol-staff-id").value,
    displayName: el("enrol-name").value,
    role: el("enrol-role").value,
    phone: el("enrol-phone").value,
    email: el("enrol-email").value,
    passphrase: el("enrol-passphrase").value,
    passphraseAgain: el("enrol-passphrase-again").value,
    hint: el("enrol-hint").value,
  };
}

async function enrolStaff(event) {
  event.preventDefault();
  const status = el("enrol-status");
  let said;
  let enrolled = false;
  try {
    said = await invoke("enrol_member", {
      credential: signerCredential() || { staffId: "", passphrase: "" },
      member: enrolMember(),
    });
    enrolled = true;
  } catch (failure) {
    // Rust sends a plain sentence; only an IPC-level failure is something else.
    said = typeof failure === "string" ? failure : failureText(failure);
  }
  // Both ways out: neither new passphrase stays on a shared screen, and the superuser's own
  // passphrase is emptied too. `false`: an enrol is not a clinical act, so the superuser does
  // not join the recent signers.
  el("enrol-passphrase").value = "";
  el("enrol-passphrase-again").value = "";
  clearSigner(false);
  if (enrolled) {
    for (const id of ["enrol-staff-id", "enrol-name", "enrol-phone", "enrol-email", "enrol-hint"]) {
      el(id).value = "";
    }
  }
  setMessage(status, said);
}

el("enrol-form").addEventListener("submit", enrolStaff);
