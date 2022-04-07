#![cfg_attr(not(feature = "std"), no_std)]

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
    use codec::{Decode, Encode};
    use frame_support::pallet_prelude::{
        DispatchResult, Hooks, IsType, TransactionSource, TransactionValidity, ValidateUnsigned,
    };
    use frame_system::ensure_signed;
    use frame_system::offchain::AppCrypto;
    use frame_system::offchain::CreateSignedTransaction;
    use frame_system::offchain::SignedPayload;
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
    pub const KEY_TYPE: KeyTypeId = KeyTypeId(*b"circ");

    const FETCH_TIMEOUT_PERIOD: u64 = 3000; // in milli-seconds
    const LOCK_TIMEOUT_EXPIRATION: u64 = FETCH_TIMEOUT_PERIOD + 1000; // in milli-seconds
    const LOCK_BLOCK_EXPIRATION: u32 = 3; // in block number

    const ONCHAIN_TX_KEY: &[u8] = b"ocw-circuits::storage::tx";
    const LOCK_KEY: &[u8] = b"ocw-circuits::lock";
    const API_ENDPOINT_GENERIC_URL: &str =
        "http://127.0.0.1:3000/interstellarpbapicircuits.SkcdApi/GenerateSkcdGenericFromIPFS";
    const API_ENDPOINT_DISPLAY_URL: &str =
        "http://127.0.0.1:3000/interstellarpbapicircuits.SkcdApi/GenerateSkcdDisplay";
    const DEFAULT_DISPLAY_WIDTH: u32 = 224;
    const DEFAULT_DISPLAY_HEIGHT: u32 = 96;

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

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        NewSkcdIpfsCid(Option<T::AccountId>, Option<Vec<u8>>),
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
            log::info!("[ocw-circuits] Hello from pallet-ocw-circuits.");

            // TODO proper job queue; eg use last_run_block_number and process all the needed ones
            let result = Self::process_if_needed(block_number);

            if let Err(e) = result {
                log::error!("[ocw-circuits] offchain_worker error: {:?}", e);
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
        pub fn submit_config_generic_signed(
            origin: OriginFor<T>,
            verilog_cid: Vec<u8>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            log::info!(
                "[ocw-circuits] submit_config_generic_signed: ({:?}, {:?})",
                sp_std::str::from_utf8(&verilog_cid).expect("verilog_cid utf8"),
                who
            );

            let copy = verilog_cid.clone();
            Self::append_or_replace_verilog_hash(Some(verilog_cid), GrpcCallKind::Generic);

            Self::deposit_event(Event::NewSkcdIpfsCid(Some(who), Some(copy)));
            Ok(())
        }

        #[pallet::weight(10000)]
        pub fn submit_config_display_signed(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            log::info!("[ocw-circuits] submit_config_display_signed: ({:?})", who);

            Self::append_or_replace_verilog_hash(None, GrpcCallKind::Display);

            Self::deposit_event(Event::NewSkcdIpfsCid(Some(who), None));
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
        Generic,
        Display,
    }

    impl Default for GrpcCallKind {
        fn default() -> Self {
            GrpcCallKind::Generic
        }
    }

    #[derive(Debug, Deserialize, Encode, Decode, Default)]
    struct IndexingData {
        // verilog_ipfs_hash only if GrpcCallKind::Generic
        // (For now) when it is GrpcCallKind::Display the corresponding Verilog are packaged with the api_circuits
        verilog_ipfs_hash: Option<Vec<u8>>,
        block_number: u32,
        grpc_kind: GrpcCallKind,
    }

    impl<T: Config> Pallet<T> {
        /// Append a new number to the tail of the list, removing an element from the head if reaching
        ///   the bounded length.
        fn append_or_replace_verilog_hash(verilog_cid: Option<Vec<u8>>, grpc_kind: GrpcCallKind) {
            let key = Self::derived_key();
            let data = IndexingData {
                verilog_ipfs_hash: verilog_cid,
                block_number: 1,
                grpc_kind: grpc_kind,
            };
            sp_io::offchain_index::set(&key, &data.encode());
        }

        /// Check if we have fetched the data before. If yes, we can use the cached version
        ///   stored in off-chain worker storage `storage`. If not, we fetch the remote info and
        ///   write the info into the storage for future retrieval.
        ///
        /// https://github.com/JoshOrndorff/recipes/blob/master/text/off-chain-workers/storage.md
        /// https://gist.github.com/spencerbh/1a150e076f4cef0ff4558642c4837050
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

            let to_process_verilog_cid = indexing_data.verilog_ipfs_hash;
            let to_process_block_number = indexing_data.block_number;

            // TODO proper job queue; or at least proper CHECK
            if to_process_block_number == 0 {
                log::info!("[ocw-circuits] nothing to do, returning...");
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

                match indexing_data.grpc_kind {
                    GrpcCallKind::Generic => match Self::call_grpc_generic(
                        &to_process_verilog_cid.expect("missing verilog_cid"),
                    ) {
                        Ok(result_ipfs_hash) => {
                            // TODO return result via tx
                            let result_ipfs_hash_str = str::from_utf8(&result_ipfs_hash)
                                .map_err(|_| <Error<T>>::DeserializeToStrError)?;
                            log::info!(
                                "[ocw-circuits] call_grpc_generic FINAL got result IPFS hash : {:x?}",
                                result_ipfs_hash_str
                            );
                        }
                        Err(err) => {
                            return Err(err);
                        }
                    },
                    GrpcCallKind::Display => match Self::call_grpc_display() {
                        Ok(result_ipfs_hash) => {
                            // TODO return result via tx
                            let result_ipfs_hash_str = str::from_utf8(&result_ipfs_hash)
                                .map_err(|_| <Error<T>>::DeserializeToStrError)?;
                            log::info!(
                                "[ocw-circuits] call_grpc_display FINAL got result IPFS hash : {:x?}",
                                result_ipfs_hash_str
                            );
                        }
                        Err(err) => {
                            return Err(err);
                        }
                    },
                }
            }
            Ok(())
        }

        /// Call the GRPC endpoint API_ENDPOINT_GENERIC_URL, encoding the request as grpc-web, and decoding the response
        ///
        /// return: a IPFS hash
        fn call_grpc_generic(verilog_cid: &Vec<u8>) -> Result<Vec<u8>, Error<T>> {
            let verilog_cid_str = sp_std::str::from_utf8(&verilog_cid)
                .expect("call_grpc_generic from_utf8")
                .to_owned();
            let input = crate::interstellarpbapicircuits::SkcdGenericFromIpfsRequest {
                verilog_cid: verilog_cid_str,
            };
            let body_bytes = ocw_common::encode_body(input);

            let (resp_bytes, resp_content_type) =
                ocw_common::fetch_from_remote_grpc_web(body_bytes, API_ENDPOINT_GENERIC_URL)
                    .map_err(|e| {
                        log::error!("[ocw-circuits] call_grpc_generic error: {:?}", e);
                        <Error<T>>::HttpFetchingError
                    })?;

            let (resp, _trailers): (
                crate::interstellarpbapicircuits::SkcdGenericFromIpfsReply,
                _,
            ) = ocw_common::decode_body(resp_bytes, resp_content_type);
            Ok(resp.skcd_cid.bytes().collect())
        }

        /// Call the GRPC endpoint API_ENDPOINT_GENERIC_URL, encoding the request as grpc-web, and decoding the response
        ///
        /// return: a IPFS hash
        fn call_grpc_display() -> Result<Vec<u8>, Error<T>> {
            let input = crate::interstellarpbapicircuits::SkcdDisplayRequest {
                width: DEFAULT_DISPLAY_WIDTH,
                height: DEFAULT_DISPLAY_HEIGHT,
            };
            let body_bytes = ocw_common::encode_body(input);

            let (resp_bytes, resp_content_type) =
                ocw_common::fetch_from_remote_grpc_web(body_bytes, API_ENDPOINT_DISPLAY_URL)
                    .map_err(|e| {
                        log::error!("[ocw-circuits] call_grpc_display error: {:?}", e);
                        <Error<T>>::HttpFetchingError
                    })?;

            let (resp, _trailers): (crate::interstellarpbapicircuits::SkcdDisplayReply, _) =
                ocw_common::decode_body(resp_bytes, resp_content_type);
            Ok(resp.skcd_cid.bytes().collect())
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
