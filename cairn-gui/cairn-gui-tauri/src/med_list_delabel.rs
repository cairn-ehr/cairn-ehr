//! The same-drug de-label offers a medication list line may show (charting slice 1b): for each
//! signable part coded to a drugref moiety, one offer per live allergy record coded to that very
//! moiety. The rule (code equality only, never a name or group) is `cairn_node`'s
//! `same_drug_heads`; this file only maps a line's parts onto it. Rust words the offer; the webview
//! draws it.
use crate::allergy_check::CodingArg;
use crate::med_chart::PartArg;
use cairn_node::allergy::check::MOIETY_SYSTEM;
use cairn_node::allergy::delabel_offer::{offer_words, same_drug_heads};
use cairn_node::allergy::thread::Resolved;
use uuid::Uuid;

/// One de-label the line may offer: the allergy head it would de-label and the drug the line
/// shows as tolerated.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DelabelOffer {
    /// The allergy head's event id.
    pub target: String,
    /// Rust's sentence for the offer.
    pub words: String,
    /// The part's coding display (what was tolerated).
    pub tolerated: String,
    /// The part's coding, carried to the de-label so it can code the tolerated drug.
    pub coding: CodingArg,
}

/// Every offer for one signable line: for each part coded [`MOIETY_SYSTEM`] (a code that does not
/// parse as a `Uuid` offers nothing), one per same-drug head, deduplicated by target (the first
/// part to meet a head keeps it). A combination thus offers only for the part that matches. Pure.
pub(crate) fn offers_for(r: &Resolved, doubted: &[Uuid], parts: &[PartArg]) -> Vec<DelabelOffer> {
    let mut offers: Vec<DelabelOffer> = vec![];
    for coding in parts.iter().filter_map(|p| p.coding.as_ref()) {
        let Some(moiety) = (coding.system == MOIETY_SYSTEM)
            .then(|| coding.code.parse::<Uuid>().ok())
            .flatten()
        else {
            continue;
        };
        for head in same_drug_heads(r, doubted, moiety) {
            let target = head.event_id.to_string();
            if offers.iter().any(|o| o.target == target) {
                continue;
            }
            offers.push(DelabelOffer {
                words: offer_words(&head.term, &coding.display, head.signer.as_deref()),
                tolerated: coding.display.clone(),
                coding: coding.clone(),
                target,
            });
        }
    }
    offers
}

#[cfg(test)]
#[path = "med_list_delabel_tests.rs"]
mod tests;
