#![cfg_attr(not(feature = "std"), no_std)]

mod interstellarpbapigarble {
    // include_bytes!(concat!(env!("OUT_DIR")), "/interstellarpbapigarble.rs");
    // include_bytes!(concat!(env!("OUT_DIR"), "/interstellarpbapigarble.rs"));
    include!(concat!(env!("OUT_DIR"), "/interstellarpbapigarble.rs"));
}

pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
    use codec::{Decode, Encode};
    use frame_support::pallet_prelude::{
        DispatchResult, Hooks, IsType, TransactionSource, TransactionValidity, ValidateUnsigned,
    };
    use frame_system::ensure_signed;
    use frame_system::offchain::AppCrypto;
    use frame_system::offchain::CreateSignedTransaction;
    use frame_system::offchain::SendSignedTransaction;
    use frame_system::offchain::SignedPayload;
    use frame_system::offchain::Signer;
    use frame_system::offchain::SigningTypes;
    use frame_system::pallet_prelude::BlockNumberFor;
    use frame_system::pallet_prelude::OriginFor;
    use serde::Deserialize;
    use sp_core::crypto::KeyTypeId;
    use sp_core::offchain::Duration;
    use sp_runtime::offchain::storage::StorageValueRef;
    use sp_runtime::offchain::storage_lock::BlockAndTime;
    use sp_runtime::offchain::storage_lock::StorageLock;
    use sp_runtime::traits::BlockNumberProvider;
    use sp_runtime::transaction_validity::InvalidTransaction;
    use sp_runtime::RuntimeDebug;
    use sp_std::borrow::ToOwned;
    use sp_std::str;
    use sp_std::vec::Vec;

    /// Defines application identifier for crypto keys of this module.
    ///
    /// Every module that deals with signatures needs to declare its unique identifier for
    /// its crypto keys.
    /// When an offchain worker is signing transactions it's going to request keys from type
    /// `KeyTypeId` via the keystore to sign the transaction.
    /// The keys can be inserted manually via RPC (see `author_insertKey`).
    pub const KEY_TYPE: KeyTypeId = KeyTypeId(*b"garb");

    const FETCH_TIMEOUT_PERIOD: u64 = 3000; // in milli-seconds
    const LOCK_TIMEOUT_EXPIRATION: u64 = FETCH_TIMEOUT_PERIOD + 1000; // in milli-seconds
    const LOCK_BLOCK_EXPIRATION: u32 = 3; // in block number

    const ONCHAIN_TX_KEY: &[u8] = b"ocw-garble::storage::tx";
    const LOCK_KEY: &[u8] = b"ocw-garble::lock";
    const API_ENDPOINT_GARBLE_URL: &str =
        "http://127.0.0.1:3001/interstellarpbapigarble.GarbleApi/GarbleIpfs";
    const API_ENDPOINT_GARBLE_STRIP_URL: &str =
        "http://127.0.0.1:3001/interstellarpbapigarble.GarbleApi/GarbleAndStripIpfs";

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
        skcd_cid: Vec<u8>,
        public: Public,
    }

    impl<T: SigningTypes> SignedPayload<T> for Payload<T::Public> {
        fn public(&self) -> T::Public {
            self.public.clone()
        }
    }

    // NOTE: pallet_tx_validation::Config b/c we want to Call its extrinsics from the internal extrinsics
    // (callback from offchain_worker)
    #[pallet::config]
    pub trait Config:
        frame_system::Config + CreateSignedTransaction<Call<Self>> + pallet_tx_validation::Config
    {
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

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        // Sent at the end of the offchain_worker(ie it is an OUTPUT)
        NewGarbledIpfsCid(Vec<u8>),
        // Strip version: one IPFS cid for the circuit, one IPFS cid for the packmsg
        NewGarbleAndStrippedIpfsCid(Vec<u8>, Vec<u8>),
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
            log::info!("[ocw-garble] Hello from pallet-ocw-garble.");

            let result = Self::process_if_needed(block_number);

            if let Err(e) = result {
                log::error!("[ocw-garble] offchain_worker error: {:?}", e);
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
        fn validate_unsigned(
            _source: TransactionSource,
            _call: &Self::Call,
        ) -> TransactionValidity {
            // TODO?
            InvalidTransaction::Call.into()
        }
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::weight(10000)]
        pub fn garble_standard_signed(origin: OriginFor<T>, skcd_cid: Vec<u8>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            log::info!(
                "[ocw-garble] garble_standard_signed: ({}, {:?})",
                sp_std::str::from_utf8(&skcd_cid).expect("skcd_cid utf8"),
                who
            );

            Self::append_or_replace_skcd_hash(skcd_cid, GrpcCallKind::GarbleStandard, None);

            Ok(())
        }

        #[pallet::weight(10000)]
        pub fn garble_and_strip_signed(
            origin: OriginFor<T>,
            skcd_cid: Vec<u8>,
            tx_msg: Vec<u8>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            log::info!(
                "[ocw-garble] garble_and_strip_signed: ({}, {:?}, {:?})",
                sp_std::str::from_utf8(&skcd_cid).expect("skcd_cid utf8"),
                sp_std::str::from_utf8(&tx_msg).expect("tx_msg utf8"),
                who
            );

            Self::append_or_replace_skcd_hash(skcd_cid, GrpcCallKind::GarbleAndStrip, Some(tx_msg));

            Ok(())
        }

        /// Called at the end of offchain_worker to publish the result
        /// Not meant to be called by a user
        // TODO use "with signed payload" and check if expected key?
        #[pallet::weight(10000)]
        pub fn callback_new_garbled_signed(
            origin: OriginFor<T>,
            pgarbled_cid: Vec<u8>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            log::info!(
                "[ocw-garble] callback_new_garbled_signed: ({:?},{:?})",
                sp_std::str::from_utf8(&pgarbled_cid).expect("skcd_cid utf8"),
                who
            );

            Self::deposit_event(Event::NewGarbledIpfsCid(pgarbled_cid));
            Ok(())
        }

        #[pallet::weight(10000)]
        pub fn callback_new_garbled_and_strip_signed(
            origin: OriginFor<T>,
            pgarbled_cid: Vec<u8>,
            packmsg_cid: Vec<u8>,
            circuit_digits: Vec<u8>,
        ) -> DispatchResult {
            let who = ensure_signed(origin.clone())?;
            log::info!(
                "[ocw-garble] callback_new_garbled_and_strip_signed: ({:?},{:?})",
                sp_std::str::from_utf8(&pgarbled_cid).expect("skcd_cid utf8"),
                who
            );

            Self::deposit_event(Event::NewGarbleAndStrippedIpfsCid(
                pgarbled_cid.clone(),
                packmsg_cid,
            ));

            // store the metadata using the pallet-tx-validation
            // (only in "garble+strip" mode b/c else it makes no sense)
            // TODO? Call?
            // pallet_tx_validation::Call::<T>::store_metadata {
            //     ipfs_cid: pgarbled_cid,
            //     circuit_digits: circuit_digits,
            // };
            pallet_tx_validation::store_metadata_aux::<T>(origin, pgarbled_cid, circuit_digits);

            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        fn derived_key() -> Vec<u8> {
            // TODO re-add block_number?
            let block_number = T::BlockNumber::default();
            block_number.using_encoded(|encoded_bn| {
                ONCHAIN_TX_KEY
                    .clone()
                    .into_iter()
                    .chain(b"/".into_iter())
                    .chain(encoded_bn)
                    .copied()
                    .collect::<Vec<u8>>()
            })
        }
    }

    #[derive(Debug, Deserialize, Encode, Decode)]
    enum GrpcCallKind {
        GarbleStandard,
        GarbleAndStrip,
    }

    // reply type for each GrpcCallKind
    enum GrpcCallReplyKind {
        GarbleStandard(crate::interstellarpbapigarble::GarbleIpfsReply),
        GarbleAndStrip(crate::interstellarpbapigarble::GarbleAndStripIpfsReply),
    }

    impl Default for GrpcCallKind {
        fn default() -> Self {
            GrpcCallKind::GarbleStandard
        }
    }

    #[derive(Debug, Deserialize, Encode, Decode, Default)]
    struct IndexingData {
        skcd_ipfs_hash: Vec<u8>,
        block_number: u32,
        grpc_kind: GrpcCallKind,
        // optional: only if GrpcCallKind::GarbleAndStrip
        tx_msg: Option<Vec<u8>>,
    }

    impl<T: Config> Pallet<T> {
        /// Append a new number to the tail of the list, removing an element from the head if reaching
        ///   the bounded length.
        fn append_or_replace_skcd_hash(
            skcd_cid: Vec<u8>,
            grpc_kind: GrpcCallKind,
            tx_msg: Option<Vec<u8>>,
        ) {
            let key = Self::derived_key();
            let data = IndexingData {
                skcd_ipfs_hash: skcd_cid,
                block_number: 1,
                grpc_kind: grpc_kind,
                tx_msg: tx_msg,
            };
            sp_io::offchain_index::set(&key, &data.encode());
        }

        /// Check if we have fetched the data before. If yes, we can use the cached version
        ///   stored in off-chain worker storage `storage`. If not, we fetch the remote info and
        ///   write the info into the storage for future retrieval.
        fn process_if_needed(_block_number: T::BlockNumber) -> Result<(), Error<T>> {
            // Reading back the off-chain indexing value. It is exactly the same as reading from
            // ocw local storage.
            //
            // IMPORTANT: writing using eg StorageValue(mutate,set,kill,take) works but DOES NOTHING
            // During the next call, the old value is there!
            // So we MUST use StorageValueRef/LocalStorage to write.
            let key = Self::derived_key();
            let oci_mem = StorageValueRef::persistent(&key);

            let indexing_data = oci_mem
                .get::<IndexingData>()
                .unwrap_or(Some(IndexingData::default()))
                .unwrap_or(IndexingData::default());

            let to_process_skcd_cid = indexing_data.skcd_ipfs_hash;
            let to_process_block_number = indexing_data.block_number;

            // TODO proper job queue; or at least proper CHECK
            if to_process_skcd_cid.is_empty() || to_process_block_number == 0 {
                log::info!("[ocw-garble] nothing to do, returning...");
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
                LOCK_KEY,
                LOCK_BLOCK_EXPIRATION,
                Duration::from_millis(LOCK_TIMEOUT_EXPIRATION),
            );

            // We try to acquire the lock here. If failed, we know the `fetch_n_parse` part inside is being
            //   executed by previous run of ocw, so the function just returns.
            if let Ok(_guard) = lock.try_lock() {
                // NOTE: remove the task from the "job queue" wether it worked or not
                // TODO better? But in this case we should only retry in case of "remote error"
                // and NOT retry if eg the given hash is not a valid IPFS hash
                //
                // DO NOT use "sp_io::offchain_index::set"!
                // We MUST use "StorageValueRef::persistent" else the value is not updated??
                oci_mem.set(&IndexingData::default());

                let result_grpc_call = match indexing_data.grpc_kind {
                    GrpcCallKind::GarbleStandard => Self::call_grpc_garble(&to_process_skcd_cid),
                    GrpcCallKind::GarbleAndStrip => Self::call_grpc_garble_and_strip(
                        &to_process_skcd_cid,
                        &indexing_data.tx_msg.expect("missing tx_msg"),
                    ),
                };

                Self::finalize_grpc_call(result_grpc_call)
            }
            Ok(())
        }

        /// Call the GRPC endpoint API_ENDPOINT_GARBLE_URL, encoding the request as grpc-web, and decoding the response
        ///
        /// return: a IPFS hash
        fn call_grpc_garble(skcd_cid: &Vec<u8>) -> Result<GrpcCallReplyKind, Error<T>> {
            let skcd_cid_str = sp_std::str::from_utf8(&skcd_cid)
                .expect("call_grpc_garble from_utf8")
                .to_owned();
            let input = crate::interstellarpbapigarble::GarbleIpfsRequest {
                skcd_cid: skcd_cid_str,
            };
            let body_bytes = ocw_common::encode_body(input);

            let (resp_bytes, resp_content_type) =
                ocw_common::fetch_from_remote_grpc_web(body_bytes, API_ENDPOINT_GARBLE_URL)
                    .map_err(|e| {
                        log::error!("[ocw-garble] call_grpc_garble error: {:?}", e);
                        <Error<T>>::HttpFetchingError
                    })?;

            let (resp, _trailers): (crate::interstellarpbapigarble::GarbleIpfsReply, _) =
                ocw_common::decode_body(resp_bytes, resp_content_type);
            Ok(GrpcCallReplyKind::GarbleStandard(resp))
        }

        /// Call the GRPC endpoint API_ENDPOINT_GARBLE_STRIP_URL, encoding the request as grpc-web, and decoding the response
        ///
        /// return: a IPFS hash
        fn call_grpc_garble_and_strip(
            skcd_cid: &Vec<u8>,
            tx_msg: &Vec<u8>,
        ) -> Result<GrpcCallReplyKind, Error<T>> {
            let skcd_cid_str = sp_std::str::from_utf8(&skcd_cid)
                .expect("call_grpc_garble_and_strip from_utf8")
                .to_owned();
            let tx_msg_str = sp_std::str::from_utf8(&tx_msg)
                .expect("call_grpc_garble_and_strip from_utf8")
                .to_owned();
            let input = crate::interstellarpbapigarble::GarbleAndStripIpfsRequest {
                skcd_cid: skcd_cid_str,
                tx_msg: tx_msg_str,
            };
            let body_bytes = ocw_common::encode_body(input);

            let (resp_bytes, resp_content_type) =
                ocw_common::fetch_from_remote_grpc_web(body_bytes, API_ENDPOINT_GARBLE_STRIP_URL)
                    .map_err(|e| {
                    log::error!("[ocw-garble] call_grpc_garble_and_strip error: {:?}", e);
                    <Error<T>>::HttpFetchingError
                })?;

            let (resp, _trailers): (crate::interstellarpbapigarble::GarbleAndStripIpfsReply, _) =
                ocw_common::decode_body(resp_bytes, resp_content_type);
            Ok(GrpcCallReplyKind::GarbleAndStrip(resp))
        }

        /// Called at the end of process_if_needed/offchain_worker
        /// Publish the result back via send_signed_transaction(and Event)
        ///
        /// param: result_grpc_call: returned by call_grpc_garble_and_strip/call_grpc_garble
        fn finalize_grpc_call(result_grpc_call: Result<GrpcCallReplyKind, Error<T>>) {
            match result_grpc_call {
                Ok(result_reply) => {
                    // Using `send_signed_transaction` associated type we create and submit a transaction
                    // representing the call we've just created.
                    // `send_signed_transaction()` return type is `Option<(Account<T>, Result<(), ()>)>`. It is:
                    //   - `None`: no account is available for sending transaction
                    //   - `Some((account, Ok(())))`: transaction is successfully sent
                    //   - `Some((account, Err(())))`: error occurred when sending the transaction
                    let signer = Signer::<T, T::AuthorityId>::all_accounts();
                    if !signer.can_sign() {
                        log::error!("[ocw-garble] No local accounts available. Consider adding one via `author_insertKey` RPC[ALTERNATIVE DEV ONLY check 'if config.offchain_worker.enabled' in service.rs]");
                    }
                    let results = signer.send_signed_transaction(|_account| match &result_reply {
                        GrpcCallReplyKind::GarbleStandard(reply) => {
                            Call::callback_new_garbled_signed {
                                pgarbled_cid: reply.pgarbled_cid.bytes().collect(),
                            }
                        }
                        GrpcCallReplyKind::GarbleAndStrip(reply) => {
                            Call::callback_new_garbled_and_strip_signed {
                                pgarbled_cid: reply.pgarbled_cid.bytes().collect(),
                                packmsg_cid: reply.packmsg_cid.bytes().collect(),
                                circuit_digits: reply
                                    .server_metadata
                                    .clone()
                                    .unwrap()
                                    .digits
                                    .to_vec(),
                            }
                        }
                    });
                    log::info!(
                        "[ocw-garble] finalize_grpc_call sent number : {:#?}",
                        results.len()
                    );
                }
                Err(err) => {
                    log::error!("[ocw-garble] finalize_grpc_call: error: {:?}", err);
                }
            }
        }
    }

    // needed for with_block_and_time_deadline()
    impl<T: Config> BlockNumberProvider for Pallet<T> {
        type BlockNumber = T::BlockNumber;

        fn current_block_number() -> Self::BlockNumber {
            <frame_system::Pallet<T>>::block_number()
        }
    }
}
