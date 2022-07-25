#![cfg_attr(not(feature = "std"), no_std)]

mod interstellarpbapicircuits {
    // include_bytes!(concat!(env!("OUT_DIR")), "/interstellarpbapicircuits.rs");
    // include_bytes!(concat!(env!("OUT_DIR"), "/interstellarpbapicircuits.rs"));
    include!(concat!(env!("OUT_DIR"), "/interstellarpbapicircuits.rs"));
}

pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
    use codec::{Decode, Encode};
    use frame_support::pallet_prelude::*;
    use frame_system::ensure_signed;
    use frame_system::offchain::AppCrypto;
    use frame_system::offchain::CreateSignedTransaction;
    use frame_system::offchain::SendSignedTransaction;
    use frame_system::offchain::SignedPayload;
    use frame_system::offchain::Signer;
    use frame_system::offchain::SigningTypes;
    use frame_system::pallet_prelude::*;
    use scale_info::prelude::*;
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

    const LOCK_TIMEOUT_EXPIRATION: u64 = 10000; // in milli-seconds
    const LOCK_BLOCK_EXPIRATION: u32 = 3; // in block number

    const ONCHAIN_TX_KEY: &[u8] = b"ocw-circuits::storage::tx";
    const LOCK_KEY: &[u8] = b"ocw-circuits::lock";
    const API_ENDPOINT_GENERIC_URL: &str =
        "/interstellarpbapicircuits.SkcdApi/GenerateSkcdGenericFromIPFS";
    const API_ENDPOINT_DISPLAY_URL: &str = "/interstellarpbapicircuits.SkcdApi/GenerateSkcdDisplay";
    // Resolutions for the "message" mode and the "pinpad" mode
    // There are no good/bad ones, it is only trial and error.
    // You SHOULD use lib_circuits's cli_display_skcd to try and find good ones.
    const DEFAULT_MESSAGE_WIDTH: u32 = 1280 / 2;
    const DEFAULT_MESSAGE_HEIGHT: u32 = 720 / 2;
    const DEFAULT_PINPAD_WIDTH: u32 = 590;
    const DEFAULT_PINPAD_HEIGHT: u32 = 50;

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

    /// Easy way to make a link b/w a "message" and "pinpad" circuits
    /// that way we can have ONE extrinsic that generates both in one call
    #[derive(
        Clone,
        Encode,
        Decode,
        Eq,
        PartialEq,
        RuntimeDebug,
        Default,
        scale_info::TypeInfo,
        MaxEncodedLen,
    )]
    pub struct DisplayCircuitsPackage {
        // 32 b/c IPFS hash is 256 bits = 32 bytes
        // But due to encoding(??) in practice it is 46 bytes(checked with debugger), and we take some margin
        message_skcd_cid: BoundedVec<u8, ConstU32<64>>,
        pinpad_skcd_cid: BoundedVec<u8, ConstU32<64>>,
    }

    /// For now it will be stored as a StorageValue but later we could use
    /// a map for various resolutions, kind of digits(7 segments vs other?), etc
    const MAX_NUMBER_PENDING_CIRCUITS_PER_ACCOUNT: u32 = 16;
    #[pallet::storage]
    pub(super) type DisplayCircuitsPackageValue<T: Config> =
        StorageValue<_, DisplayCircuitsPackage, ValueQuery>;

    #[pallet::pallet]
    #[pallet::generate_store(pub(super) trait Store)]
    pub struct Pallet<T>(_);

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        // Sent at the end of the offchain_worker(ie it is an OUTPUT)
        NewSkcdIpfsCid(Vec<u8>),
        // Display version: one IPFS cid for the message, one IPFS cid for the pinpad
        NewDisplayCircuitsPackageIpfsCid(Vec<u8>, Vec<u8>),
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

            Self::append_or_replace_verilog_hash(Some(verilog_cid), GrpcCallKind::Generic);

            Ok(())
        }

        #[pallet::weight(10000)]
        pub fn submit_config_display_circuits_package_signed(
            origin: OriginFor<T>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            log::info!(
                "[ocw-circuits] submit_config_display_circuits_package_signed: ({:?})",
                who
            );

            Self::append_or_replace_verilog_hash(None, GrpcCallKind::Display);

            Ok(())
        }

        /// Called at the end of offchain_worker to publish the result
        /// Not meant to be called by a user
        // TODO use "with signed payload" and check if expected key?
        #[pallet::weight(10000)]
        pub fn callback_new_skcd_signed(origin: OriginFor<T>, skcd_cid: Vec<u8>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            log::info!(
                "[ocw-circuits] callback_new_skcd_signed: ({:?},{:?})",
                sp_std::str::from_utf8(&skcd_cid).expect("skcd_cid utf8"),
                who
            );

            Self::deposit_event(Event::NewSkcdIpfsCid(skcd_cid));
            Ok(())
        }

        #[pallet::weight(10000)]
        pub fn callback_new_display_circuits_package_signed(
            origin: OriginFor<T>,
            message_cid: Vec<u8>,
            pinpad_cid: Vec<u8>,
        ) -> DispatchResult {
            let who = ensure_signed(origin.clone())?;
            log::info!(
                "[ocw-circuits] callback_new_display_circuits_package_signed: ({:?},{:?} for {:?})",
                sp_std::str::from_utf8(&message_cid).expect("skcd_cid utf8"),
                sp_std::str::from_utf8(&pinpad_cid).expect("skcd_cid utf8"),
                who
            );

            Self::deposit_event(Event::NewDisplayCircuitsPackageIpfsCid(
                message_cid.clone(),
                pinpad_cid.clone(),
            ));

            // and update the current "reference" circuits package
            <DisplayCircuitsPackageValue<T>>::set(DisplayCircuitsPackage {
                message_skcd_cid: TryInto::<BoundedVec<u8, ConstU32<64>>>::try_into(message_cid)
                    .unwrap(),
                pinpad_skcd_cid: TryInto::<BoundedVec<u8, ConstU32<64>>>::try_into(pinpad_cid)
                    .unwrap(),
            });

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

    // reply type for each GrpcCallKind
    enum GrpcCallReplyKind {
        Generic(crate::interstellarpbapicircuits::SkcdGenericFromIpfsReply),
        Display(crate::interstellarpbapicircuits::SkcdDisplayReply),
    }

    #[derive(Debug, Deserialize, Encode, Decode, Default)]
    struct IndexingData {
        // verilog_ipfs_hash only if GrpcCallKind::Generic
        // (For now) when it is GrpcCallKind::Display the corresponding Verilog are packaged in the repo api_circuits
        // = in "display mode" the Verilog are hardcoded, NOT passed dynamically via IPFS; contrary to "generic mode"
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

                let result_grpc_call = match indexing_data.grpc_kind {
                    GrpcCallKind::Generic => Self::call_grpc_generic(
                        &to_process_verilog_cid.expect("missing verilog_cid"),
                    ),
                    GrpcCallKind::Display => Self::call_grpc_display(true),
                };

                Self::finalize_grpc_call(result_grpc_call)
            }
            Ok(())
        }

        /// Call the GRPC endpoint API_ENDPOINT_GENERIC_URL, encoding the request as grpc-web, and decoding the response
        ///
        /// return: a IPFS hash
        fn call_grpc_generic(verilog_cid: &Vec<u8>) -> Result<GrpcCallReplyKind, Error<T>> {
            let verilog_cid_str = sp_std::str::from_utf8(&verilog_cid)
                .expect("call_grpc_generic from_utf8")
                .to_owned();
            let input = crate::interstellarpbapicircuits::SkcdGenericFromIpfsRequest {
                verilog_cid: verilog_cid_str,
            };
            let body_bytes = ocw_common::encode_body(input);

            // construct the full endpoint URI using:
            // - dynamic "URI root" from env
            // - hardcoded API_ENDPOINT_GENERIC_URL from "const" in this file
            #[cfg(feature = "std")]
            let uri_root = std::env::var("aaa").unwrap();
            #[cfg(not(feature = "std"))]
            let uri_root = "PLACEHOLDER_no_std";
            let endpoint = format!("{}{}", uri_root, API_ENDPOINT_GENERIC_URL);

            let (resp_bytes, resp_content_type) =
                ocw_common::fetch_from_remote_grpc_web(body_bytes, &endpoint).map_err(|e| {
                    log::error!("[ocw-circuits] call_grpc_generic error: {:?}", e);
                    <Error<T>>::HttpFetchingError
                })?;

            let (resp, _trailers): (
                crate::interstellarpbapicircuits::SkcdGenericFromIpfsReply,
                _,
            ) = ocw_common::decode_body(resp_bytes, resp_content_type);
            Ok(GrpcCallReplyKind::Generic(resp))
        }

        /// Call the GRPC endpoint API_ENDPOINT_GENERIC_URL, encoding the request as grpc-web, and decoding the response
        ///
        /// return: a IPFS hash
        fn call_grpc_display(is_message: bool) -> Result<GrpcCallReplyKind, Error<T>> {
            let input = if is_message {
                crate::interstellarpbapicircuits::SkcdDisplayRequest {
                    width: DEFAULT_MESSAGE_WIDTH,
                    height: DEFAULT_MESSAGE_HEIGHT,
                    digits_bboxes: vec![
                        // first digit bbox --------------------------------------------
                        0.25_f32, 0.1_f32, 0.45_f32, 0.9_f32,
                        // second digit bbox -------------------------------------------
                        0.55_f32, 0.1_f32, 0.75_f32, 0.9_f32,
                    ],
                }
            } else {
                // IMPORTANT: by convention the "pinpad" is 10 digits, placed horizontally(side by side)
                // DO NOT change the layout, else wallet-app will NOT display the pinpad correctly!
                // That is b/c this layout in treated as a "texture atlas" so the positions MUST be known.
                // Ideally the positions SHOULD be passed from here all the way into the serialized .pgarbled/.packmsg
                // but this NOT YET the case.

                // 10 digits, 4 corners(vertices) per digit
                let mut digits_bboxes: Vec<f32> = Vec::with_capacity(10 * 4);
                /*
                for (int i = 0; i < 10; i++) {
                    digits_bboxes.emplace_back(0.1f * i, 0.0f, 0.1f * (i + 1), 1.0f);
                }
                */
                for i in 0..10 {
                    digits_bboxes.append(
                        vec![
                            0.1_f32 * i as f32,
                            0.0_f32,
                            0.1_f32 * (i + 1) as f32,
                            1.0_f32,
                        ]
                        .as_mut(),
                    );
                }

                crate::interstellarpbapicircuits::SkcdDisplayRequest {
                    width: DEFAULT_PINPAD_WIDTH,
                    height: DEFAULT_PINPAD_HEIGHT,
                    digits_bboxes: digits_bboxes,
                }
            };

            let body_bytes = ocw_common::encode_body(input);

            // construct the full endpoint URI using:
            // - dynamic "URI root" from env
            // - hardcoded API_ENDPOINT_DISPLAY_URL from "const" in this file
            #[cfg(feature = "std")]
            let uri_root = std::env::var("INTERSTELLAR_URI_ROOT_API_CIRCUITS").unwrap();
            #[cfg(not(feature = "std"))]
            let uri_root = "PLACEHOLDER_no_std";
            let endpoint = format!("{}{}", uri_root, API_ENDPOINT_DISPLAY_URL);

            let (resp_bytes, resp_content_type) =
                ocw_common::fetch_from_remote_grpc_web(body_bytes, &endpoint).map_err(|e| {
                    log::error!("[ocw-circuits] call_grpc_display error: {:?}", e);
                    <Error<T>>::HttpFetchingError
                })?;

            let (resp, _trailers): (crate::interstellarpbapicircuits::SkcdDisplayReply, _) =
                ocw_common::decode_body(resp_bytes, resp_content_type);
            Ok(GrpcCallReplyKind::Display(resp))
        }

        /// Called at the end of process_if_needed/offchain_worker
        /// Publish the result back via send_signed_transaction(and Event)
        ///
        /// param: result_grpc_call: returned by call_grpc_display/call_grpc_generic
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
                        log::error!("[ocw-circuits] No local accounts available. Consider adding one via `author_insertKey` RPC[ALTERNATIVE DEV ONLY check 'if config.offchain_worker.enabled' in service.rs]");
                    }

                    let results = signer.send_signed_transaction(|_account| match &result_reply {
                        GrpcCallReplyKind::Generic(reply) => Call::callback_new_skcd_signed {
                            skcd_cid: reply.skcd_cid.bytes().collect(),
                        },
                        GrpcCallReplyKind::Display(reply) => {
                            Call::callback_new_display_circuits_package_signed {
                                message_cid: reply.skcd_cid.bytes().collect(),
                                pinpad_cid: reply.skcd_cid.bytes().collect(),
                            }
                        }
                    });

                    log::info!(
                        "[ocw-circuits] callback_new_skcd_signed sent number : {:#?}",
                        results.len()
                    );
                }
                Err(err) => {
                    log::error!("[ocw-circuits] finalize_grpc_call: error: {:?}", err);
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
