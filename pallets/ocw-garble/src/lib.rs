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
    use frame_support::pallet_prelude::*;
    use frame_support::traits::Randomness;
    use frame_system::ensure_signed;
    use frame_system::offchain::AppCrypto;
    use frame_system::offchain::CreateSignedTransaction;
    use frame_system::offchain::SendSignedTransaction;
    use frame_system::offchain::Signer;
    use frame_system::pallet_prelude::*;
    use rand::seq::SliceRandom;
    use rand::Rng;
    use rand_chacha::{rand_core::SeedableRng, ChaChaRng};
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
    pub const KEY_TYPE: KeyTypeId = KeyTypeId(*b"garb");

    const LOCK_TIMEOUT_EXPIRATION: u64 = 10000; // in milli-seconds
    const LOCK_BLOCK_EXPIRATION: u32 = 3; // in block number

    const ONCHAIN_TX_KEY: &[u8] = b"ocw-garble::storage::tx";
    const LOCK_KEY: &[u8] = b"ocw-garble::lock";
    const API_ENDPOINT_GARBLE_URL: &str = "/interstellarpbapigarble.GarbleApi/GarbleIpfs";
    const API_ENDPOINT_GARBLE_STRIP_URL: &str =
        "/interstellarpbapigarble.GarbleApi/GarbleAndStripIpfs";

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

    // NOTE: pallet_tx_validation::Config b/c we want to Call its extrinsics from the internal extrinsics
    // (callback from offchain_worker)
    #[pallet::config]
    pub trait Config:
        frame_system::Config
        + CreateSignedTransaction<Call<Self>>
        + pallet_tx_validation::Config
        + pallet_ocw_circuits::Config
    {
        /// The overarching event type.
        type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;
        /// The overarching dispatch call type.
        type Call: From<Call<Self>>;
        /// The identifier type for an offchain worker.
        type AuthorityId: AppCrypto<Self::Public, Self::Signature>;
        type MyRandomness: Randomness<Self::Hash, Self::BlockNumber>;
    }

    /// Easy way to make a link b/w a "message" and "pinpad" circuits
    /// that way we can have ONE extrinsic that generates both in one call
    ///
    /// It SHOULD roughly mirror pallet_ocw_circuits::DisplaySkcdPackage
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
    pub struct DisplayStrippedCircuitsPackage {
        // 32 b/c IPFS hash is 256 bits = 32 bytes
        // But due to encoding(??) in practice it is 46 bytes(checked with debugger), and we take some margin
        pub message_pgarbled_cid: BoundedVec<u8, ConstU32<64>>,
        pub message_packmsg_cid: BoundedVec<u8, ConstU32<64>>,
        pub pinpad_pgarbled_cid: BoundedVec<u8, ConstU32<64>>,
        pub pinpad_packmsg_cid: BoundedVec<u8, ConstU32<64>>,
        message_nb_digits: u32,
    }

    type PendingCircuitsType = BoundedVec<
        DisplayStrippedCircuitsPackage,
        ConstU32<MAX_NUMBER_PENDING_CIRCUITS_PER_ACCOUNT>,
    >;

    /// Store account_id -> list(ipfs_cids);
    /// That represents the "list of pending txs" for a given Account
    const MAX_NUMBER_PENDING_CIRCUITS_PER_ACCOUNT: u32 = 16;
    #[pallet::storage]
    #[pallet::getter(fn get_pending_circuits_for_account)]
    pub(super) type AccountToPendingCircuitsMap<T: Config> = StorageMap<
        _,
        Twox128,
        // key: AccountId
        T::AccountId,
        PendingCircuitsType,
        ValueQuery,
    >;

    #[pallet::storage]
    pub(super) type Nonce<T: Config> = StorageValue<_, u64, ValueQuery>;

    #[pallet::pallet]
    #[pallet::generate_store(pub(super) trait Store)]
    pub struct Pallet<T>(_);

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        // Sent at the end of the offchain_worker(ie it is an OUTPUT)
        NewGarbledIpfsCid(Vec<u8>),
        // Strip version: (one IPFS cid for the circuit, one IPFS cid for the packmsg), for both mesage and pinpad
        NewGarbleAndStrippedIpfsCid(Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>),
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

    impl<T: Config> Pallet<T> {
        fn get_and_increment_nonce() -> Vec<u8> {
            let nonce = <Nonce<T>>::get();
            <Nonce<T>>::put(nonce.wrapping_add(1));
            nonce.encode()
        }
    }

    // TODO SHOULD be equally weighted!
    // fn u8_to_digit(input: &u8) -> u8 {
    //     match input {
    //         0..=25 => 0,
    //         26..=50 => 1,
    //         51..=75 => 2,
    //         76..=100 => 3,
    //         101..=125 => 4,
    //         126..=150 => 5,
    //         151..=175 => 6,
    //         176..=200 => 7,
    //         201..=225 => 8,
    //         226..=255 => 9,
    //     }
    // }

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

            Self::append_or_replace_skcd_hash(
                GrpcCallKind::GarbleStandard,
                Some(skcd_cid),
                None,
                None,
                None,
                None,
                None,
            );

            Ok(())
        }

        #[pallet::weight(10000)]
        pub fn garble_and_strip_display_circuits_package_signed(
            origin: OriginFor<T>,
            tx_msg: Vec<u8>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            log::info!(
                "[ocw-garble] garble_and_strip_display_circuits_package_signed: ({:?} for {:?})",
                sp_std::str::from_utf8(&tx_msg).expect("tx_msg utf8"),
                who
            );

            // read DisplayCircuitsPackageValue directly from ocw-circuits
            let display_circuits_package =
                pallet_ocw_circuits::Pallet::<T>::get_display_circuits_package()
                    .expect("display_circuits_package not ready!");
            log::info!(
                "[ocw-garble] display_circuits_package: ({:?},{:?}) ({:?},{:?})",
                sp_std::str::from_utf8(&display_circuits_package.message_skcd_cid)
                    .expect("message_skcd_cid utf8"),
                display_circuits_package.message_skcd_server_metadata_nb_digits,
                sp_std::str::from_utf8(&display_circuits_package.pinpad_skcd_cid)
                    .expect("pinpad_skcd_cid utf8"),
                display_circuits_package.pinpad_skcd_server_metadata_nb_digits,
            );

            // Generate random digits
            // cf https://github.com/paritytech/substrate/blob/master/frame/lottery/src/lib.rs#L506
            let nonce = Self::get_and_increment_nonce();
            let (random_seed, _) = T::MyRandomness::random(&nonce);
            // random_seed is a Hash so 256 -> 32 u8 is fine
            // typically we need (2-4) digits(NOT u8) for the message
            // and 10 digits(NOT u8) for the pinpad
            // so we have more than enough
            // TODO we could(SHOULD) split "random_seed" 4 bits by 4 bits b/c that is enough for [0-10] range
            let random_seed = <[u8; 32]>::decode(&mut random_seed.as_ref())
                .expect("secure hashes should always be bigger than u32; qed");
            log::debug!("[ocw-garble] random_seed: {:?}", random_seed,);

            // let mut random_numbers2: Vec<_> = random_seed.iter().map(|x| u8_to_digit(x)).collect();
            // log::info!("[ocw-garble] digits2: {:?}", random_numbers2,);

            // https://github.com/paritytech/substrate/blob/master/frame/society/src/lib.rs#L1420
            // TODO is ChaChaRng secure? (or at least good enough)
            let mut rng = ChaChaRng::from_seed(random_seed);
            let mut pinpad_digits: Vec<u8> = (0..10).collect();
            pinpad_digits.shuffle(&mut rng);
            log::info!("[ocw-garble] pinpad_digits: {:?}", pinpad_digits,);

            // MUST SHUFFLE the pinpad digits, NOT randomize them
            // each [0-10] MUST be in the final "digits"
            let message_digits: Vec<u8> = (0..2).map(|_| rng.gen_range(0..10)).collect();
            log::info!("[ocw-garble] message_digits: {:?}", message_digits,);

            Self::append_or_replace_skcd_hash(
                GrpcCallKind::GarbleAndStrip,
                // optional: only if GrpcCallKind::GarbleStandard
                None,
                // optional: only if GrpcCallKind::GarbleAndStrip
                Some(display_circuits_package.message_skcd_cid.to_vec()),
                Some(display_circuits_package.pinpad_skcd_cid.to_vec()),
                Some(tx_msg),
                Some(message_digits),
                Some(pinpad_digits),
            );

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
            message_pgarbled_cid: Vec<u8>,
            message_packmsg_cid: Vec<u8>,
            message_digits: Vec<u8>,
            pinpad_pgarbled_cid: Vec<u8>,
            pinpad_packmsg_cid: Vec<u8>,
            pinpad_digits: Vec<u8>,
        ) -> DispatchResult {
            let who = ensure_signed(origin.clone())?;
            log::info!(
                "[ocw-garble] callback_new_garbled_and_strip_signed: ({:?},{:?}) ({:?},{:?}) for {:?}",
                sp_std::str::from_utf8(&message_pgarbled_cid).expect("message_pgarbled_cid utf8"),
                sp_std::str::from_utf8(&message_packmsg_cid).expect("message_packmsg_cid utf8"),
                sp_std::str::from_utf8(&pinpad_pgarbled_cid).expect("pinpad_pgarbled_cid utf8"),
                sp_std::str::from_utf8(&pinpad_packmsg_cid).expect("pinpad_packmsg_cid utf8"),
                who
            );

            Self::deposit_event(Event::NewGarbleAndStrippedIpfsCid(
                message_pgarbled_cid.clone(),
                message_packmsg_cid.clone(),
                pinpad_pgarbled_cid.clone(),
                pinpad_packmsg_cid.clone(),
            ));

            // store the metadata using the pallet-tx-validation
            // (only in "garble+strip" mode b/c else it makes no sense)
            // TODO? Call?
            // pallet_tx_validation::Call::<T>::store_metadata {
            //     ipfs_cid: pgarbled_cid,
            //     circuit_digits: circuit_digits,
            // };
            pallet_tx_validation::store_metadata_aux::<T>(
                origin,
                message_pgarbled_cid.clone(),
                message_digits.clone(),
                pinpad_digits.clone(),
            )
            .expect("store_metadata_aux failed!");

            // and update our internal map of pending circuits for the given account
            // this is USED via RPC by the app, not directly!
            // "append if exists, create if not"
            // TODO done in two steps, is there a way to do it atomically?
            let mut current_pending_circuits: PendingCircuitsType =
                <AccountToPendingCircuitsMap<T>>::try_get(&who).unwrap_or_default();
            current_pending_circuits
                .try_push(DisplayStrippedCircuitsPackage {
                    message_pgarbled_cid: TryInto::<BoundedVec<u8, ConstU32<64>>>::try_into(
                        message_pgarbled_cid,
                    )
                    .unwrap(),
                    message_packmsg_cid: TryInto::<BoundedVec<u8, ConstU32<64>>>::try_into(
                        message_packmsg_cid,
                    )
                    .unwrap(),
                    pinpad_pgarbled_cid: TryInto::<BoundedVec<u8, ConstU32<64>>>::try_into(
                        pinpad_pgarbled_cid,
                    )
                    .unwrap(),
                    pinpad_packmsg_cid: TryInto::<BoundedVec<u8, ConstU32<64>>>::try_into(
                        pinpad_packmsg_cid,
                    )
                    .unwrap(),
                    message_nb_digits: message_digits.len().try_into().unwrap(),
                })
                .unwrap();
            <AccountToPendingCircuitsMap<T>>::insert(who, current_pending_circuits);

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
        /// two reply b/c we call the same endpoint twice: one for message, then one for pinpad
        /// param Vec<u8> = "digits"; generated randomly in "garble_and_strip_display_circuits_package_signed"
        ///   and passed all the way around
        GarbleAndStrip(
            crate::interstellarpbapigarble::GarbleAndStripIpfsReply,
            Vec<u8>,
            crate::interstellarpbapigarble::GarbleAndStripIpfsReply,
            Vec<u8>,
        ),
    }

    impl Default for GrpcCallKind {
        fn default() -> Self {
            GrpcCallKind::GarbleStandard
        }
    }

    #[derive(Debug, Deserialize, Encode, Decode, Default)]
    struct IndexingData {
        block_number: u32,
        grpc_kind: GrpcCallKind,
        // optional: only if GrpcCallKind::GarbleStandard
        skcd_ipfs_cid: Option<Vec<u8>>,
        // optional: only if GrpcCallKind::GarbleAndStrip
        message_skcd_ipfs_cid: Option<Vec<u8>>,
        pinpad_skcd_ipfs_cid: Option<Vec<u8>>,
        tx_msg: Option<Vec<u8>>,
        message_digits: Option<Vec<u8>>,
        pinpad_digits: Option<Vec<u8>>,
    }

    impl<T: Config> Pallet<T> {
        /// Append a new number to the tail of the list, removing an element from the head if reaching
        ///   the bounded length.
        fn append_or_replace_skcd_hash(
            grpc_kind: GrpcCallKind,
            // optional: only if GrpcCallKind::GarbleStandard
            skcd_cid: Option<Vec<u8>>,
            // optional: only if GrpcCallKind::GarbleAndStrip
            message_skcd_ipfs_cid: Option<Vec<u8>>,
            pinpad_skcd_ipfs_cid: Option<Vec<u8>>,
            tx_msg: Option<Vec<u8>>,
            message_digits: Option<Vec<u8>>,
            pinpad_digits: Option<Vec<u8>>,
        ) {
            let key = Self::derived_key();
            let data = IndexingData {
                block_number: 1,
                grpc_kind: grpc_kind,
                // optional: only if GrpcCallKind::GarbleStandard
                skcd_ipfs_cid: skcd_cid,
                // optional: only if GrpcCallKind::GarbleAndStrip
                message_skcd_ipfs_cid: message_skcd_ipfs_cid,
                pinpad_skcd_ipfs_cid: pinpad_skcd_ipfs_cid,
                tx_msg: tx_msg,
                message_digits: message_digits,
                pinpad_digits: pinpad_digits,
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

            let to_process_block_number = indexing_data.block_number;

            // TODO proper job queue; or at least proper CHECK
            if to_process_block_number == 0 {
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
                    GrpcCallKind::GarbleStandard => Self::call_grpc_garble(
                        indexing_data.skcd_ipfs_cid.expect("missing skcd_ipfs_cid"),
                    ),
                    GrpcCallKind::GarbleAndStrip => Self::call_grpc_garble_and_strip(
                        indexing_data
                            .message_skcd_ipfs_cid
                            .expect("missing message_skcd_ipfs_cid"),
                        indexing_data
                            .pinpad_skcd_ipfs_cid
                            .expect("missing pinpad_skcd_ipfs_cid"),
                        indexing_data.tx_msg.expect("missing tx_msg"),
                        indexing_data
                            .message_digits
                            .expect("missing message_digits"),
                        indexing_data.pinpad_digits.expect("missing pinpad_digits"),
                    ),
                };

                Self::finalize_grpc_call(result_grpc_call)
            }
            Ok(())
        }

        /// Call the GRPC endpoint API_ENDPOINT_GARBLE_URL, encoding the request as grpc-web, and decoding the response
        ///
        /// return: a IPFS hash
        fn call_grpc_garble(skcd_cid: Vec<u8>) -> Result<GrpcCallReplyKind, Error<T>> {
            let skcd_cid_str = sp_std::str::from_utf8(&skcd_cid)
                .expect("call_grpc_garble from_utf8")
                .to_owned();
            let input = crate::interstellarpbapigarble::GarbleIpfsRequest {
                skcd_cid: skcd_cid_str,
            };
            let body_bytes = ocw_common::encode_body(input);

            // construct the full endpoint URI using:
            // - dynamic "URI root" from env
            // - hardcoded API_ENDPOINT_GARBLE_URL from "const" in this file
            #[cfg(feature = "std")]
            let uri_root = std::env::var("INTERSTELLAR_URI_ROOT_API_GARBLE").unwrap();
            #[cfg(not(feature = "std"))]
            let uri_root = "PLACEHOLDER_no_std";
            let endpoint = format!("{}{}", uri_root, API_ENDPOINT_GARBLE_URL);

            let (resp_bytes, resp_content_type) =
                ocw_common::fetch_from_remote_grpc_web(body_bytes, &endpoint).map_err(|e| {
                    log::error!("[ocw-garble] call_grpc_garble error: {:?}", e);
                    <Error<T>>::HttpFetchingError
                })?;

            let (resp, _trailers): (crate::interstellarpbapigarble::GarbleIpfsReply, _) =
                ocw_common::decode_body(resp_bytes, resp_content_type);
            Ok(GrpcCallReplyKind::GarbleStandard(resp))
        }

        /// Regroup the 2 calls to API_ENDPOINT_GARBLE_STRIP_URL in one
        fn call_grpc_garble_and_strip(
            message_skcd_ipfs_cid: Vec<u8>,
            pinpad_skcd_ipfs_cid: Vec<u8>,
            tx_msg: Vec<u8>,
            message_digits: Vec<u8>,
            pinpad_digits: Vec<u8>,
        ) -> Result<GrpcCallReplyKind, Error<T>> {
            /// INTERNAL: call API_ENDPOINT_GARBLE_STRIP_URL for one circuits
            fn call_grpc_garble_and_strip_one<T>(
                skcd_cid: Vec<u8>,
                tx_msg: Vec<u8>,
                digits: Vec<u8>,
            ) -> Result<crate::interstellarpbapigarble::GarbleAndStripIpfsReply, Error<T>>
            {
                let skcd_cid_str = sp_std::str::from_utf8(&skcd_cid)
                    .expect("call_grpc_garble_and_strip from_utf8")
                    .to_owned();
                let tx_msg_str = sp_std::str::from_utf8(&tx_msg)
                    .expect("call_grpc_garble_and_strip from_utf8")
                    .to_owned();
                let input = crate::interstellarpbapigarble::GarbleAndStripIpfsRequest {
                    skcd_cid: skcd_cid_str,
                    tx_msg: tx_msg_str,
                    server_metadata: Some(crate::interstellarpbapigarble::CircuitServerMetadata {
                        digits: digits.clone().into(),
                    }),
                };
                let body_bytes = ocw_common::encode_body(input);

                // construct the full endpoint URI using:
                // - dynamic "URI root" from env
                // - hardcoded API_ENDPOINT_GARBLE_STRIP_URL from "const" in this file
                #[cfg(feature = "std")]
                let uri_root = std::env::var("INTERSTELLAR_URI_ROOT_API_GARBLE").unwrap();
                #[cfg(not(feature = "std"))]
                let uri_root = "PLACEHOLDER_no_std";
                let endpoint = format!("{}{}", uri_root, API_ENDPOINT_GARBLE_STRIP_URL);

                let (resp_bytes, resp_content_type) =
                    ocw_common::fetch_from_remote_grpc_web(body_bytes, &endpoint).map_err(|e| {
                        log::error!("[ocw-garble] call_grpc_garble_and_strip error: {:?}", e);
                        <Error<T>>::HttpFetchingError
                    })?;

                let (resp, _trailers): (
                    crate::interstellarpbapigarble::GarbleAndStripIpfsReply,
                    _,
                ) = ocw_common::decode_body(resp_bytes, resp_content_type);
                Ok(resp)
            }

            // TODO pass correct params for pinpad and message
            let message_reply = call_grpc_garble_and_strip_one::<T>(
                message_skcd_ipfs_cid,
                tx_msg,
                message_digits.clone(),
            );
            let pinpad_reply = call_grpc_garble_and_strip_one::<T>(
                pinpad_skcd_ipfs_cid,
                vec![],
                pinpad_digits.clone(),
            );

            // TODO pass correct params for pinpad and message
            Ok(GrpcCallReplyKind::GarbleAndStrip(
                message_reply.expect("message_reply failed!"),
                message_digits,
                pinpad_reply.expect("pinpad_reply failed!"),
                pinpad_digits,
            ))
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
                    let signer = Signer::<T, <T as Config>::AuthorityId>::all_accounts();
                    if !signer.can_sign() {
                        log::error!("[ocw-garble] No local accounts available. Consider adding one via `author_insertKey` RPC[ALTERNATIVE DEV ONLY check 'if config.offchain_worker.enabled' in service.rs]");
                    }

                    let results = signer.send_signed_transaction(|_account| match &result_reply {
                        GrpcCallReplyKind::GarbleStandard(reply) => {
                            Call::callback_new_garbled_signed {
                                pgarbled_cid: reply.pgarbled_cid.bytes().collect(),
                            }
                        }
                        GrpcCallReplyKind::GarbleAndStrip(
                            message_reply,
                            message_digits,
                            pinpad_reply,
                            pinpad_digits,
                        ) => Call::callback_new_garbled_and_strip_signed {
                            message_pgarbled_cid: message_reply.pgarbled_cid.bytes().collect(),
                            message_packmsg_cid: message_reply.packmsg_cid.bytes().collect(),
                            message_digits: message_digits.to_vec(),
                            pinpad_pgarbled_cid: pinpad_reply.pgarbled_cid.bytes().collect(),
                            pinpad_packmsg_cid: pinpad_reply.packmsg_cid.bytes().collect(),
                            pinpad_digits: pinpad_digits.to_vec(),
                        },
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
