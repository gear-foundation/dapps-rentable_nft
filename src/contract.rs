use gear_lib::non_fungible_token::{io::NFTTransfer, nft_core::*, state::*, token::*};
use gear_lib_derive::{NFTCore, NFTMetaState, NFTStateKeeper};
use gmeta::Metadata;
use gstd::{
    errors::Result as GstdResult,
    exec::{self, block_timestamp},
    msg,
    prelude::*,
    ActorId, MessageId,
};
use hashbrown::HashMap;
use nft_io::{InitNft, IoNft, NFTAction, NFTEvent, NFTMetadata, UserInfo};
use primitive_types::{H256, U256};

static mut CONTRACT: Option<Nft> = None;

#[derive(Debug, Default, NFTStateKeeper, NFTCore, NFTMetaState)]
pub struct Nft {
    #[NFTStateField]
    pub token: NFTState,
    pub token_id: TokenId,
    pub owner: ActorId,
    pub transactions: HashMap<H256, NFTEvent>,
    pub users_info: HashMap<TokenId, UserInfo>,
}

#[no_mangle]
unsafe extern "C" fn init() {
    let config: InitNft = msg::load().expect("Unable to decode InitNft");
    if config.royalties.is_some() {
        config.royalties.as_ref().expect("Unable to g").validate();
    }
    let nft = Nft {
        token: NFTState {
            name: config.name,
            symbol: config.symbol,
            base_uri: config.base_uri,
            royalties: config.royalties,
            ..Default::default()
        },
        owner: msg::source(),
        ..Default::default()
    };
    CONTRACT = Some(nft);
}

#[no_mangle]
unsafe extern "C" fn handle() {
    let action: NFTAction = msg::load().expect("Could not load NFTAction");
    let nft = CONTRACT.get_or_insert(Default::default());
    match action {
        NFTAction::Mint {
            transaction_id,
            token_metadata,
        } => {
            msg::reply(
                nft.process_transaction(transaction_id, |nft| {
                    NFTEvent::Transfer(MyNFTCore::mint(nft, token_metadata))
                }),
                0,
            )
            .expect("Error during replying with `NFTEvent::Transfer`");
        }
        NFTAction::Burn {
            transaction_id,
            token_id,
        } => {
            msg::reply(
                nft.process_transaction(transaction_id, |nft| {
                    NFTEvent::Transfer(NFTCore::burn(nft, token_id))
                }),
                0,
            )
            .expect("Error during replying with `NFTEvent::Transfer`");
        }
        NFTAction::Transfer {
            transaction_id,
            to,
            token_id,
        } => {
            msg::reply(
                nft.process_transaction(transaction_id, |nft| {
                    NFTEvent::Transfer(NFTCore::transfer(nft, &to, token_id))
                }),
                0,
            )
            .expect("Error during replying with `NFTEvent::Transfer`");
        }
        NFTAction::TransferPayout {
            transaction_id,
            to,
            token_id,
            amount,
        } => {
            msg::reply(
                nft.process_transaction(transaction_id, |nft| {
                    NFTEvent::TransferPayout(NFTCore::transfer_payout(nft, &to, token_id, amount))
                }),
                0,
            )
            .expect("Error during replying with `NFTEvent::TransferPayout`");
        }
        NFTAction::NFTPayout { owner, amount } => {
            msg::reply(
                NFTEvent::NFTPayout(NFTCore::nft_payout(nft, &owner, amount)),
                0,
            )
            .expect("Error during replying with `NFTEvent::NFTPayout`");
        }
        NFTAction::Approve {
            transaction_id,
            to,
            token_id,
        } => {
            msg::reply(
                nft.process_transaction(transaction_id, |nft| {
                    NFTEvent::Approval(NFTCore::approve(nft, &to, token_id))
                }),
                0,
            )
            .expect("Error during replying with `NFTEvent::Approval`");
        }
        NFTAction::Owner { token_id } => {
            msg::reply(
                NFTEvent::Owner {
                    owner: NFTCore::owner_of(nft, token_id),
                    token_id,
                },
                0,
            )
            .expect("Error during replying with `NFTEvent::Owner`");
        }
        NFTAction::IsApproved { to, token_id } => {
            msg::reply(
                NFTEvent::IsApproved {
                    to,
                    token_id,
                    approved: NFTCore::is_approved_to(nft, &to, token_id),
                },
                0,
            )
            .expect("Error during replying with `NFTEvent::IsApproved`");
        }
        NFTAction::DelegatedApprove {
            transaction_id,
            message,
            signature,
        } => {
            msg::reply(
                nft.process_transaction(transaction_id, |nft| {
                    NFTEvent::Approval(NFTCore::delegated_approve(nft, message, signature))
                }),
                0,
            )
            .expect("Error during replying with `NFTEvent::Approval`");
        }
        NFTAction::Clear { transaction_hash } => nft.clear(transaction_hash),
        NFTAction::SetUser {
            token_id,
            address,
            expires,
            transaction_id,
        } => {
            nft.set_user(address, token_id, expires);

            let event = nft.process_transaction(transaction_id, |_nft| NFTEvent::UpdateUser {
                token_id,
                address,
                expires,
            });
            msg::reply(event, 0).expect("Error during replying with `NFTEvent::SetUser`");
        }
        NFTAction::UserOf { token_id } => {
            let address = nft.user_of(&token_id);
            let payload = NFTEvent::UserOf { address };
            msg::reply(payload, 0).expect("Error during replying with `NFTEvent::UserOf`");
        }
        NFTAction::UserExpires { token_id } => {
            let expires = nft.user_expires(&token_id);
            let payload = NFTEvent::UserExpires { expires };
            msg::reply(payload, 0).expect("Error during replying with `NFTEvent::UserExpires`");
        }
    };
}

pub trait MyNFTCore: NFTCore {
    fn mint(&mut self, token_metadata: TokenMetadata) -> NFTTransfer;
}

impl MyNFTCore for Nft {
    fn mint(&mut self, token_metadata: TokenMetadata) -> NFTTransfer {
        let transfer = NFTCore::mint(self, &msg::source(), self.token_id, Some(token_metadata));
        self.token_id = self.token_id.saturating_add(U256::one());
        transfer
    }
}

impl Nft {
    fn process_transaction(
        &mut self,
        transaction_id: u64,
        action: impl FnOnce(&mut Nft) -> NFTEvent,
    ) -> NFTEvent {
        let transaction_hash = get_hash(&msg::source(), transaction_id);

        if let Some(nft_event) = self.transactions.get(&transaction_hash) {
            nft_event.clone()
        } else {
            let nft_event = action(self);

            self.transactions
                .insert(transaction_hash, nft_event.clone());

            nft_event
        }
    }

    fn clear(&mut self, transaction_hash: H256) {
        assert_eq!(
            msg::source(),
            exec::program_id(),
            "Not allowed to clear transactions"
        );
        self.transactions.remove(&transaction_hash);
    }

    fn set_user(&mut self, address: ActorId, token_id: TokenId, expires: u64) {
        self.assert_zero_address(&address);

        let owner = &self.owner;

        // is Approved or Owner
        if !self.is_approved_to(&msg::source(), token_id) {
            self.assert_owner(owner);
        }

        self.users_info
            .entry(token_id)
            .and_modify(|user_info| user_info.expires = expires)
            .or_insert(UserInfo { address, expires });
    }

    fn user_of(&self, token_id: &TokenId) -> ActorId {
        if let Some(user_info) = self.users_info.get(token_id) {
            if user_info.expires < block_timestamp() {
                return user_info.address;
            }
        }

        ActorId::zero()
    }

    fn user_expires(&self, token_id: &TokenId) -> u64 {
        if let Some(user_info) = self.users_info.get(token_id) {
            user_info.expires
        } else {
            0u64
        }
    }
}

fn static_mut_state() -> &'static Nft {
    unsafe { CONTRACT.get_or_insert(Default::default()) }
}

fn common_state() -> <NFTMetadata as Metadata>::State {
    static_mut_state().into()
}

#[no_mangle]
extern "C" fn state() {
    reply(common_state())
        .expect("Failed to encode or reply with `<NFTMetadata as Metadata>::State` from `state()`");
}

fn reply(payload: impl Encode) -> GstdResult<MessageId> {
    msg::reply(payload, 0)
}

pub fn get_hash(account: &ActorId, transaction_id: u64) -> H256 {
    let account: [u8; 32] = (*account).into();
    let transaction_id = transaction_id.to_be_bytes();
    sp_core_hashing::blake2_256(&[account.as_slice(), transaction_id.as_slice()].concat()).into()
}

impl From<&Nft> for IoNft {
    fn from(value: &Nft) -> Self {
        let Nft {
            token,
            token_id,
            owner,
            transactions,
            users_info,
        } = value;

        let transactions = transactions
            .iter()
            .map(|(key, event)| (*key, event.clone()))
            .collect();

        let users_info = users_info.iter().map(|(id, info)| (*id, *info)).collect();

        Self {
            token: token.into(),
            token_id: *token_id,
            owner: *owner,
            transactions,
            users_info,
        }
    }
}
