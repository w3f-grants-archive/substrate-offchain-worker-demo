#![cfg_attr(not(feature = "std"), no_std)]

use bytes::Buf;
use bytes::BufMut;
// use codec::{Decode, Encode};
use frame_support::traits::Get;
use frame_system::{
    self as system,
    offchain::{
        AppCrypto, CreateSignedTransaction, SendSignedTransaction, SendUnsignedTransaction,
        SignedPayload, Signer, SigningTypes, SubmitTransaction,
    },
};
use sp_core::{crypto::KeyTypeId, hexdisplay::AsBytesRef};
use sp_runtime::{
    offchain::{
        http,
        storage::{MutateStorageError, StorageRetrievalError, StorageValueRef},
        Duration,
    },
    traits::Zero,
    transaction_validity::{InvalidTransaction, TransactionValidity, ValidTransaction},
    RuntimeDebug,
};
use sp_std::borrow::ToOwned;
use sp_std::vec;
use sp_std::vec::Vec;

use prost::Message;

mod interstellarpbapicircuits {
    // include_bytes!(concat!(env!("OUT_DIR")), "/interstellarpbapicircuits.rs");
    // include_bytes!(concat!(env!("OUT_DIR"), "/interstellarpbapicircuits.rs"));
    include!(concat!(env!("OUT_DIR"), "/interstellarpbapicircuits.rs"));
}

pub use pallet::*;

#[cfg(test)]
mod tests;

#[frame_support::pallet]
pub mod pallet {
    //! A demonstration of an offchain worker that sends onchain callbacks
    use codec::{Decode, Encode};
    use core::convert::TryInto;
    use frame_support::pallet_prelude::*;
    use frame_system::{
        offchain::{
            AppCrypto, CreateSignedTransaction, SendSignedTransaction, SendUnsignedTransaction,
            SignedPayload, Signer, SigningTypes, SubmitTransaction,
        },
        pallet_prelude::*,
    };
    use sp_core::crypto::KeyTypeId;
    use sp_runtime::{
        offchain::{
            http,
            storage::StorageValueRef,
            storage_lock::{BlockAndTime, StorageLock},
            Duration,
        },
        traits::BlockNumberProvider,
        transaction_validity::{
            InvalidTransaction, TransactionSource, TransactionValidity, ValidTransaction,
        },
        RuntimeDebug,
    };
    use sp_std::{collections::vec_deque::VecDeque, prelude::*, str};

    use serde::{Deserialize, Deserializer};

    /// Defines application identifier for crypto keys of this module.
    ///
    /// Every module that deals with signatures needs to declare its unique identifier for
    /// its crypto keys.
    /// When an offchain worker is signing transactions it's going to request keys from type
    /// `KeyTypeId` via the keystore to sign the transaction.
    /// The keys can be inserted manually via RPC (see `author_insertKey`).
    pub const KEY_TYPE: KeyTypeId = KeyTypeId(*b"demo");
    const NUM_VEC_LEN: usize = 10;
    /// The type to sign and send transactions.
    const UNSIGNED_TXS_PRIORITY: u64 = 100;

    const FETCH_TIMEOUT_PERIOD: u64 = 3000; // in milli-seconds
    const LOCK_TIMEOUT_EXPIRATION: u64 = FETCH_TIMEOUT_PERIOD + 1000; // in milli-seconds
    const LOCK_BLOCK_EXPIRATION: u32 = 3; // in block number

    /// Based on the above `KeyTypeId` we need to generate a pallet-specific crypto type wrapper.
    /// We can utilize the supported crypto kinds (`sr25519`, `ed25519` and `ecdsa`) and augment
    /// them with the pallet-specific identifier.
    pub mod crypto {
        use crate::KEY_TYPE;
        use sp_core::sr25519::Signature as Sr25519Signature;
        use sp_runtime::{
            app_crypto::{app_crypto, sr25519},
            traits::Verify,
            MultiSignature, MultiSigner,
        };
        use sp_std::prelude::*;

        app_crypto!(sr25519, KEY_TYPE);

        pub struct TestAuthId;
        // implemented for runtime
        impl frame_system::offchain::AppCrypto<MultiSigner, MultiSignature> for TestAuthId {
            type RuntimeAppPublic = Public;
            type GenericSignature = sp_core::sr25519::Signature;
            type GenericPublic = sp_core::sr25519::Public;
        }

        // implemented for mock runtime in test
        impl
            frame_system::offchain::AppCrypto<
                <Sr25519Signature as Verify>::Signer,
                Sr25519Signature,
            > for TestAuthId
        {
            type RuntimeAppPublic = Public;
            type GenericSignature = sp_core::sr25519::Signature;
            type GenericPublic = sp_core::sr25519::Public;
        }
    }

    #[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, scale_info::TypeInfo)]
    pub struct Payload<Public> {
        verilog_cid: Vec<u8>,
        public: Public,
    }

    impl<T: SigningTypes> SignedPayload<T> for Payload<T::Public> {
        fn public(&self) -> T::Public {
            self.public.clone()
        }
    }

    #[derive(Debug, Deserialize, Encode, Decode, Default)]
    struct IndexingData(Vec<u8>, u64);

    pub fn de_string_to_bytes<'de, D>(de: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: &str = Deserialize::deserialize(de)?;
        Ok(s.as_bytes().to_vec())
    }

    #[pallet::config]
    pub trait Config: frame_system::Config + CreateSignedTransaction<Call<Self>> {
        /// The overarching event type.
        type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;
        /// The overarching dispatch call type.
        type Call: From<Call<Self>>;
        /// The identifier type for an offchain worker.
        type AuthorityId: AppCrypto<Self::Public, Self::Signature>;
    }

    #[pallet::pallet]
    #[pallet::generate_store(pub(super) trait Store)]
    pub struct Pallet<T>(_);

    // The pallet's runtime storage items.
    // https://substrate.dev/docs/en/knowledgebase/runtime/storage
    #[pallet::storage]
    #[pallet::getter(fn numbers)]
    // Learn more about declaring storage items:
    // https://substrate.dev/docs/en/knowledgebase/runtime/storage#declaring-storage-items
    pub type VerilogIpfsCids<T> = StorageValue<_, VecDeque<Vec<u8>>, ValueQuery>;

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        // TODO NewDisplayConfig(Option<T::AccountId>, u32, u32),
        NewSkcdIpfsCid(Option<T::AccountId>, Vec<u8>),
    }

    // Errors inform users that something went wrong.
    #[pallet::error]
    pub enum Error<T> {
        // Error returned when not sure which ocw function to executed
        UnknownOffchainMux,

        // Error returned when making signed transactions in off-chain worker
        NoLocalAcctForSigning,
        OffchainSignedTxError,

        // Error returned when making unsigned transactions in off-chain worker
        OffchainUnsignedTxError,

        // Error returned when making unsigned transactions with signed payloads in off-chain worker
        OffchainUnsignedTxSignedPayloadError,

        // Error returned when fetching github info
        HttpFetchingError,
        DeserializeToObjError,
        DeserializeToStrError,
    }

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        /// Offchain Worker entry point.
        ///
        /// By implementing `fn offchain_worker` you declare a new offchain worker.
        /// This function will be called when the node is fully synced and a new best block is
        /// succesfuly imported.
        /// Note that it's not guaranteed for offchain workers to run on EVERY block, there might
        /// be cases where some blocks are skipped, or for some the worker runs twice (re-orgs),
        /// so the code should be able to handle that.
        /// You can use `Local Storage` API to coordinate runs of the worker.
        fn offchain_worker(block_number: T::BlockNumber) {
            log::info!("[example] Hello from pallet-ocw.");

            // Here we are showcasing various techniques used when running off-chain workers (ocw)
            // 1. Sending signed transaction from ocw
            // 2. Sending unsigned transaction from ocw
            // 3. Sending unsigned transactions with signed payloads from ocw
            // 4. Fetching JSON via http requests in ocw
            // const TX_TYPES: u32 = 4;
            // let modu = block_number
            //     .try_into()
            //     .map_or(TX_TYPES, |bn: usize| (bn as u32) % TX_TYPES);
            // let result = match modu {
            // 0 => Self::offchain_signed_tx(block_number),
            // 1 => Self::offchain_unsigned_tx(block_number),
            // 2 => Self::offchain_unsigned_tx_signed_payload(block_number),
            // 3 => Self::fetch_remote_info(),
            // _ => Err(Error::<T>::UnknownOffchainMux),
            // };

            let result = Self::fetch_remote_info();

            if let Err(e) = result {
                log::error!("[example] offchain_worker error: {:?}", e);
            }
        }
    }

    #[pallet::validate_unsigned]
    impl<T: Config> ValidateUnsigned for Pallet<T> {
        type Call = Call<T>;

        /// Validate unsigned call to this module.
        ///
        /// By default unsigned transactions are disallowed, but implementing the validator
        /// here we make sure that some particular calls (the ones produced by offchain worker)
        /// are being whitelisted and marked as valid.
        fn validate_unsigned(_source: TransactionSource, call: &Self::Call) -> TransactionValidity {
            // let valid_tx = |provide| {
            //     ValidTransaction::with_tag_prefix("ocw-demo")
            //         .priority(UNSIGNED_TXS_PRIORITY)
            //         .and_provides([&provide])
            //         .longevity(3)
            //         .propagate(true)
            //         .build()
            // };

            // TODO
            InvalidTransaction::Call.into()

            // match call {
            //     Call::submit_number_unsigned { number: _number } => {
            //         valid_tx(b"submit_number_unsigned".to_vec())
            //     }
            //     Call::submit_number_unsigned_with_signed_payload {
            //         ref payload,
            //         ref signature,
            //     } => {
            //         if !SignedPayload::<T>::verify::<T::AuthorityId>(payload, signature.clone()) {
            //             return InvalidTransaction::BadProof.into();
            //         }
            //         valid_tx(b"submit_number_unsigned_with_signed_payload".to_vec())
            //     }
            //     _ => InvalidTransaction::Call.into(),
            // }
        }
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::weight(10000)]
        pub fn submit_config_generic_signed(
            origin: OriginFor<T>,
            verilog_cid: Vec<u8>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            log::info!(
                "[example] submit_config_generic_signed: ({}, {:?})",
                sp_std::str::from_utf8(&verilog_cid).expect("verilog_cid utf8"),
                who
            );

            let copy = verilog_cid.clone();
            Self::append_or_replace_verilog_hash(verilog_cid);

            Self::deposit_event(Event::NewSkcdIpfsCid(Some(who), copy));
            Ok(())
        }

        #[pallet::weight(10000)]
        pub fn submit_config_generic_unsigned(
            origin: OriginFor<T>,
            verilog_cid: Vec<u8>,
        ) -> DispatchResult {
            let _ = ensure_none(origin)?;
            log::info!(
                "[example] submit_config_generic_unsigned: {}",
                sp_std::str::from_utf8(&verilog_cid).expect("verilog_cid utf8")
            );

            let copy = verilog_cid.clone();
            Self::append_or_replace_verilog_hash(verilog_cid);

            Self::deposit_event(Event::NewSkcdIpfsCid(None, copy));
            Ok(())
        }

        // #[pallet::weight(10000)]
        // #[allow(unused_variables)]
        // pub fn submit_number_unsigned_with_signed_payload(
        //     origin: OriginFor<T>,
        //     payload: Payload<T::Public>,
        //     signature: T::Signature,
        // ) -> DispatchResult {
        //     let _ = ensure_none(origin)?;
        //     // we don't need to verify the signature here because it has been verified in
        //     //   `validate_unsigned` function when sending out the unsigned tx.
        //     let Payload { verilog_cid, public } = payload;
        //     log::info!(
        //         "[example] submit_number_unsigned_with_signed_payload: ({}, {:?})",
        //         verilog_cid,
        //         public
        //     );
        //     Self::append_or_replace_verilog_hash(verilog_cid);

        //     Self::deposit_event(Event::New(None, verilog_cid));
        //     Ok(())
        // }
    }

    impl<T: Config> Pallet<T> {
        /// Append a new number to the tail of the list, removing an element from the head if reaching
        ///   the bounded length.
        fn append_or_replace_verilog_hash(verilog_cid: Vec<u8>) {
            VerilogIpfsCids::<T>::mutate(|verilog_cids| {
                if verilog_cids.len() == NUM_VEC_LEN {
                    let _ = verilog_cids.pop_front();
                }
                verilog_cids.push_back(verilog_cid);
                log::info!("[example] VerilogIpfsCids vector: {:?}", verilog_cids);
            });

            // TODO refacto: move call to Self::deposit_event in here
        }

        /// Check if we have fetched the data before. If yes, we can use the cached version
        ///   stored in off-chain worker storage `storage`. If not, we fetch the remote info and
        ///   write the info into the storage for future retrieval.
        fn fetch_remote_info() -> Result<(), Error<T>> {
            // // Create a reference to Local Storage value.
            // // Since the local storage is common for all offchain workers, it's a good practice
            // // to prepend our entry with the pallet name.
            // let s_info = StorageValueRef::persistent(b"offchain-demo::hn-info");

            // // Local storage is persisted and shared between runs of the offchain workers,
            // // offchain workers may run concurrently. We can use the `mutate` function to
            // // write a storage entry in an atomic fashion.
            // //
            // // With a similar API as `StorageValue` with the variables `get`, `set`, `mutate`.
            // // We will likely want to use `mutate` to access
            // // the storage comprehensively.
            // //
            // if let Ok(Some(info)) = s_info.get::<HackerNewsInfo>() {
            //     // hn-info has already been fetched. Return early.
            //     log::info!("[example] cached hn-info: {:?}", info);
            //     return Ok(());
            // }

            let verilog_cids_to_process = <VerilogIpfsCids<T>>::get();
            if verilog_cids_to_process.is_empty() {
                log::info!("[example] nothing to do, returning...");
                return Ok(());
            }

            // Since off-chain storage can be accessed by off-chain workers from multiple runs, it is important to lock
            //   it before doing heavy computations or write operations.
            //
            // There are four ways of defining a lock:
            //   1) `new` - lock with default time and block exipration
            //   2) `with_deadline` - lock with default block but custom time expiration
            //   3) `with_block_deadline` - lock with default time but custom block expiration
            //   4) `with_block_and_time_deadline` - lock with custom time and block expiration
            // Here we choose the most custom one for demonstration purpose.
            let mut lock = StorageLock::<BlockAndTime<Self>>::with_block_and_time_deadline(
                b"offchain-demo::lock",
                LOCK_BLOCK_EXPIRATION,
                Duration::from_millis(LOCK_TIMEOUT_EXPIRATION),
            );

            // get the first one from the list to be processed
            let verilog_cid = VerilogIpfsCids::<T>::mutate(|verilog_cids| verilog_cids.pop_front());
            let verilog_cid = verilog_cid.expect("verilog_cid.expect");

            // We try to acquire the lock here. If failed, we know the `fetch_n_parse` part inside is being
            //   executed by previous run of ocw, so the function just returns.
            if let Ok(_guard) = lock.try_lock() {
                match Self::fetch_n_parse(verilog_cid) {
                    Ok(info) => {
                        // TODO return result via tx
                        // s_info.set(&info);
                        log::info!("[example] FINAL got result IPFS hash : {:x?}", info);
                    }
                    Err(err) => {
                        return Err(err);
                    }
                }
            }
            Ok(())
        }

        /// Fetch from remote and deserialize the JSON to a struct
        fn fetch_n_parse(verilog_cid: Vec<u8>) -> Result<Vec<u8>, Error<T>> {
            let resp_bytes = Self::fetch_from_remote(verilog_cid).map_err(|e| {
                log::error!("[example] fetch_from_remote error: {:?}", e);
                <Error<T>>::HttpFetchingError
            })?;

            let resp_str =
                str::from_utf8(&resp_bytes).map_err(|_| <Error<T>>::DeserializeToStrError)?;
            // Print out our fetched JSON string
            log::info!("[example] fetch_n_parse: {}", resp_str);

            Ok(resp_str.encode())
        }

        /// This function uses the `offchain::http` API to query the remote endpoint information,
        ///   and returns the JSON response as vector of bytes.
        fn fetch_from_remote(verilog_cid: Vec<u8>) -> Result<Vec<u8>, http::Error> {
            // We want to keep the offchain worker execution time reasonable, so we set a hard-coded
            // deadline to 2s to complete the external call.
            // You can also wait idefinitely for the response, however you may still get a timeout
            // coming from the host machine.
            let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(2_000));

            // TODO get from payload(ie tx)
            let body = crate::encode_body_generic(verilog_cid);
            log::info!("[example] sending body b64: {}", base64::encode(&body));

            // Initiate an external HTTP GET request.
            // This is using high-level wrappers from `sp_runtime`, for the low-level calls that
            // you can find in `sp_io`. The API is trying to be similar to `reqwest`, but
            // since we are running in a custom WASM execution environment we can't simply
            // import the library here.
            //
            // cf https://github.com/hyperium/tonic/blob/master/tonic-web/tests/integration/tests/grpc_web.rs
            // syntax = "proto3";
            // package test;
            // service Test {
            //		rpc SomeRpc(Input) returns (Output);
            // -> curl http://127.0.0.1:3000/test.Test/SomeRpc
            //
            // NOTE application/grpc-web == application/grpc-web+proto
            //      application/grpc-web-text = base64
            //
            // eg:
            // printf '\x00\x00\x00\x00\x05\x08\xe0\x01\x10\x60' | curl -skv -H "Content-Type: application/grpc-web+proto" -H "X-Grpc-Web: 1" -H "Accept: application/grpc-web-text+proto" -X POST --data-binary @- http://127.0.0.1:3000/interstellarpbapigarble.SkcdApi/GenerateSkcdDisplay
            let request = http::Request::post(
                "http://127.0.0.1:3000/interstellarpbapicircuits.SkcdApi/GenerateSkcdGenericFromIPFS",
                vec![body],
            )
            .add_header("Content-Type", "application/grpc-web")
            .add_header("X-Grpc-Web", "1");

            // We set the deadline for sending of the request, note that awaiting response can
            // have a separate deadline. Next we send the request, before that it's also possible
            // to alter request headers or stream body content in case of non-GET requests.
            let pending = request
                .deadline(deadline)
                .send()
                .map_err(|_| http::Error::IoError)?;

            // The request is already being processed by the host, we are free to do anything
            // else in the worker (we can send multiple concurrent requests too).
            // At some point however we probably want to check the response though,
            // so we can block current thread and wait for it to finish.
            // Note that since the request is being driven by the host, we don't have to wait
            // for the request to have it complete, we will just not read the response.
            let mut response = pending
                .try_wait(deadline)
                .map_err(|_| http::Error::DeadlineReached)??;

            log::info!("[example] status code: {}", response.code);
            let mut headers_it = response.headers().into_iter();
            while headers_it.next() {
                let header = headers_it.current().unwrap();
                log::info!("[example] header: {} {}", header.0, header.1);
            }

            // Let's check the status code before we proceed to reading the response.
            if response.code != 200 {
                log::warn!("[example] Unexpected status code: {}", response.code);
                return Err(http::Error::Unknown);
            }

            // TODO handle like parse_price
            let body_bytes = response.body().collect::<bytes::Bytes>();
            let (reply, trailers) = crate::decode_body_generic(body_bytes, "application/grpc-web");

            log::info!(
                "[example] Got gRPC trailers: {}",
                sp_std::str::from_utf8(&trailers).expect("trailers")
            );
            log::info!("[example] Got IPFS hash: {}", reply.skcd_cid);

            Ok(reply.skcd_cid.bytes().collect())
        }

        // fn offchain_signed_tx(block_number: T::BlockNumber) -> Result<(), Error<T>> {
        //     // We retrieve a signer and check if it is valid.
        //     //   Since this pallet only has one key in the keystore. We use `any_account()1 to
        //     //   retrieve it. If there are multiple keys and we want to pinpoint it, `with_filter()` can be chained,
        //     let signer = Signer::<T, T::AuthorityId>::any_account();

        //     // Translating the current block number to number and submit it on-chain
        //     let number: u64 = block_number.try_into().unwrap_or(0);

        //     // `result` is in the type of `Option<(Account<T>, Result<(), ()>)>`. It is:
        //     //   - `None`: no account is available for sending transaction
        //     //   - `Some((account, Ok(())))`: transaction is successfully sent
        //     //   - `Some((account, Err(())))`: error occured when sending the transaction
        //     let result = signer.send_signed_transaction(|_acct|
        // 		// This is the on-chain function
        // 		Call::submit_number_signed { number });

        //     // Display error if the signed tx fails.
        //     if let Some((acc, res)) = result {
        //         if res.is_err() {
        //             log::error!("[example] failure: offchain_signed_tx: tx sent: {:?}", acc.id);
        //             return Err(<Error<T>>::OffchainSignedTxError);
        //         }
        //         // Transaction is sent successfully
        //         return Ok(());
        //     }

        //     // The case of `None`: no account is available for sending
        //     log::error!("[example] No local account available");
        //     Err(<Error<T>>::NoLocalAcctForSigning)
        // }

        // fn offchain_unsigned_tx(block_number: T::BlockNumber) -> Result<(), Error<T>> {
        //     let number: u64 = block_number.try_into().unwrap_or(0);
        //     let call = Call::submit_number_unsigned { number };

        //     // `submit_unsigned_transaction` returns a type of `Result<(), ()>`
        //     //   ref: https://substrate.dev/rustdocs/v2.0.0/frame_system/offchain/struct.SubmitTransaction.html#method.submit_unsigned_transaction
        //     SubmitTransaction::<T, Call<T>>::submit_unsigned_transaction(call.into()).map_err(
        //         |_| {
        //             log::error!("[example] Failed in offchain_unsigned_tx");
        //             <Error<T>>::OffchainUnsignedTxError
        //         },
        //     )
        // }

        // fn offchain_unsigned_tx_signed_payload(
        //     block_number: T::BlockNumber,
        // ) -> Result<(), Error<T>> {
        //     // Retrieve the signer to sign the payload
        //     let signer = Signer::<T, T::AuthorityId>::any_account();

        //     let number: u64 = block_number.try_into().unwrap_or(0);

        //     // `send_unsigned_transaction` is returning a type of `Option<(Account<T>, Result<(), ()>)>`.
        //     //   Similar to `send_signed_transaction`, they account for:
        //     //   - `None`: no account is available for sending transaction
        //     //   - `Some((account, Ok(())))`: transaction is successfully sent
        //     //   - `Some((account, Err(())))`: error occured when sending the transaction
        //     if let Some((_, res)) = signer.send_unsigned_transaction(
        //         |acct| Payload {
        //             number,
        //             public: acct.public.clone(),
        //         },
        //         |payload, signature| Call::submit_number_unsigned_with_signed_payload {
        //             payload,
        //             signature,
        //         },
        //     ) {
        //         return res.map_err(|_| {
        //             log::error!("[example] Failed in offchain_unsigned_tx_signed_payload");
        //             <Error<T>>::OffchainUnsignedTxSignedPayloadError
        //         });
        //     }

        //     // The case of `None`: no account is available for sending
        //     log::error!("[example] No local account available");
        //     Err(<Error<T>>::NoLocalAcctForSigning)
        // }
    }

    impl<T: Config> BlockNumberProvider for Pallet<T> {
        type BlockNumber = T::BlockNumber;

        fn current_block_number() -> Self::BlockNumber {
            <frame_system::Pallet<T>>::block_number()
        }
    }
}

// we CAN NOT just send the raw encoded protobuf(eg using SkcdDisplayRequest{}.encode())
// b/c that returns errors like
// "protocol error: received message with invalid compression flag: 8 (valid flags are 0 and 1), while sending request"
// "tonic-web: Invalid byte 45, offset 0"
// https://github.com/hyperium/tonic/blob/01e5be508051eebf19c233d48b57797a17331383/tonic-web/tests/integration/tests/grpc_web.rs#L93
// also: https://github.com/grpc/grpc-web/issues/152
fn encode_body_generic(verilog_cid: Vec<u8>) -> bytes::Bytes {
    let verilog_cid_str = sp_std::str::from_utf8(&verilog_cid)
        .expect("encode_body_generic from_utf8")
        .to_owned();

    let input = interstellarpbapicircuits::SkcdGenericFromIpfsRequest {
        verilog_cid: verilog_cid_str,
    };

    let mut buf = bytes::BytesMut::with_capacity(1024);
    buf.reserve(5);
    unsafe {
        buf.advance_mut(5);
    }

    input.encode(&mut buf).unwrap();

    let len = buf.len() - 5;
    {
        let mut buf = &mut buf[..5];
        buf.put_u8(0);
        buf.put_u32(len as u32);
    }

    buf.split_to(len + 5).freeze()
}

fn decode_body_generic(
    body_bytes: bytes::Bytes,
    content_type: &str,
) -> (
    interstellarpbapicircuits::SkcdGenericFromIpfsReply,
    bytes::Bytes,
) {
    let mut body = body_bytes;

    if content_type == "application/grpc-web-text+proto" {
        body = base64::decode(body).unwrap().into()
    }

    body.advance(1);
    let len = body.get_u32();
    let msg = interstellarpbapicircuits::SkcdGenericFromIpfsReply::decode(
        &mut body.split_to(len as usize),
    )
    .expect("decode");
    body.advance(5);

    (msg, body)
}
