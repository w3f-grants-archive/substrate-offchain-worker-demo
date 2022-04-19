#![cfg_attr(not(feature = "std"), no_std)]

/// Edit this file to define custom logic or remove it if it is not needed.
/// Learn more about FRAME and the core library of Substrate FRAME pallets:
/// <https://docs.substrate.io/v3/runtime/frame>
pub use pallet::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

#[frame_support::pallet]
pub mod pallet {
    use frame_support::pallet_prelude::*;
    use frame_system::pallet_prelude::*;
    use sp_std::vec::Vec;

    /// Configure the pallet by specifying the parameters and types on which it depends.
    #[pallet::config]
    pub trait Config: frame_system::Config + 'static {
        /// Because this pallet emits events, it depends on the runtime's definition of an event.
        type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;
    }

    // TODO proper structs instead of tuples for the StorageMap(both key and value)
    // #[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, Default, scale_info::TypeInfo)]
    // pub struct CircuitServerMetadata {
    //     // 20 digits max for now; the current pratical max is 10(10 digits on a pinpad)
    //     // no real point in displaying even more than 4 on a "message display"
    //     // TODO BoundedVec
    //     // digits: BoundedVec<u8, ConstU32<20>>,
    //     digits: Vec<u8>,
    // }

    // #[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, Default)]
    // pub struct CircuitServerMetadataKey<T: Config> {
    //     // 20 digits max for now; the current pratical max is 10(10 digits on a pinpad)
    //     // no real point in displaying even more than 4 on a "message display"
    //     account_id: T::AccountId,
    //     /// 32 b/c IPFS hash is 256 bits = 32 bytes
    //     /// Yes we could abuse T::AccountId which is also 32 bytes but that would not be clean
    //     /// TODO BoundedVec
    //     // ipfs_cid: BoundedVec<u8, ConstU32<32>>,
    //     ipfs_cid: Vec<u8>,
    // }

    // impl MaxEncodedLen for CircuitServerMetadata {
    //     fn max_encoded_len() -> usize {
    //         32
    //     }
    // }

    // impl<T: Config> MaxEncodedLen for CircuitServerMetadataKey<T> {
    //     fn max_encoded_len() -> usize {
    //         32
    //     }
    // }

    // impl<T: Config> TypeInfo for CircuitServerMetadataKey<T>
    // where
    //     T: TypeInfo + 'static,
    // {
    //     type Identity = Self;

    //     fn type_info() -> scale_info::Type {
    //         scale_info::Type::builder()
    //             .path(scale_info::Path::new("CircuitServerMetadataKey", module_path!()))
    //             // .type_params(vec![scale_info::MetaType::new::<T>()])
    //             .type_params(vec![scale_info::TypeParameter::new(
    //                 "T",
    //                 Some(scale_info::meta_type::<CircuitServerMetadataKey<T>>()),
    //             )])
    //             .composite(
    //                 scale_info::build::Fields::named()
    //                     .field(|f| f.ty::<T>().name("account_id").type_name("T::AccountId"))
    //                     .field(|f| f.ty::<u64>().name("ipfs_cid").type_name("Vec<u8>")),
    //             )
    //     }
    // }

    /// Store ipfs_hash -> CircuitServerMetadata; typically at least the OTP/digits/permutation
    /// This will be checked against user input to pass/fail the current tx
    // #[pallet::storage]
    // #[pallet::getter(fn circuit_server_metadata_map)]
    // pub(super) type CircuitServerMetadataMap<T: Config> =
    //     StorageMap<_, Twox128, CircuitServerMetadataKey<T>, CircuitServerMetadata, ValueQuery>;
    #[pallet::storage]
    #[pallet::getter(fn circuit_server_metadata_map)]
    pub(super) type CircuitServerMetadataMap<T: Config> = StorageMap<
        _,
        Twox128,
        // key: (AccountId, IPFS hash)
        // 32 b/c IPFS hash is 256 bits = 32 bytes
        // But due to encoding(??) in practice it is 46 bytes(checked with debugger)
        (T::AccountId, BoundedVec<u8, ConstU32<64>>),
        (BoundedVec<u8, ConstU32<20>>,),
        ValueQuery,
    >;

    #[pallet::pallet]
    #[pallet::generate_store(pub(super) trait Store)]
    pub struct Pallet<T>(_);

    // Pallets use events to inform users when important changes are made.
    // https://docs.substrate.io/v3/runtime/events-and-errors
    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// One of those is emitted at the end of the tx validation
        TxPass {
            account_id: T::AccountId,
        },
        TxFail {
            account_id: T::AccountId,
        },
        /// DEBUG ONLY
        DEBUGNewDigitsSet {
            circuit_digits: Vec<u8>,
        },
    }

    // Errors inform users that something went wrong.
    #[pallet::error]
    pub enum Error<T> {
        TxWrongInputGiven,
        /// Errors should have helpful documentation associated with them.
        StorageOverflow,
    }

    pub fn store_metadata_aux<T: Config>(
        origin: OriginFor<T>,
        ipfs_cid: Vec<u8>,
        circuit_digits: Vec<u8>,
    ) -> DispatchResult {
        // Check that the extrinsic was signed and get the signer.
        // This function will return an error if the extrinsic is not signed.
        // https://docs.substrate.io/v3/runtime/origins
        let who = ensure_signed(origin)?;

        crate::Pallet::<T>::deposit_event(Event::DEBUGNewDigitsSet {
            circuit_digits: circuit_digits.clone(),
        });

        // Update storage.
        <CircuitServerMetadataMap<T>>::insert(
            (
                who,
                TryInto::<BoundedVec<u8, ConstU32<64>>>::try_into(ipfs_cid).unwrap(),
            ),
            (TryInto::<BoundedVec<u8, ConstU32<20>>>::try_into(circuit_digits).unwrap(),),
        );

        Ok(())
    }

    // Dispatchable functions allows users to interact with the pallet and invoke state changes.
    // These functions materialize as "extrinsics", which are often compared to transactions.
    // Dispatchable functions must be annotated with a weight and must return a DispatchResult.
    #[pallet::call]
    impl<T: Config> Pallet<T> {
        // TODO remove call? how to properly handle calling store_metadata_aux from pallet-ocw-garble???
        #[pallet::weight(10_000 + T::DbWeight::get().writes(1))]
        pub fn store_metadata(
            origin: OriginFor<T>,
            ipfs_cid: Vec<u8>,
            circuit_digits: Vec<u8>,
        ) -> DispatchResult {
            store_metadata_aux::<T>(origin, ipfs_cid, circuit_digits)
        }

        // NOTE: for now this extrinsic is called from the front-end so input_digits is ascii
        // ie when giving "35" in the text box, we get [51,53]
        #[pallet::weight(10_000 + T::DbWeight::get().writes(1))]
        pub fn check_input(
            origin: OriginFor<T>,
            ipfs_cid: Vec<u8>,
            input_digits: Vec<u8>,
        ) -> DispatchResult {
            // Check that the extrinsic was signed and get the signer.
            // This function will return an error if the extrinsic is not signed.
            // https://docs.substrate.io/v3/runtime/origins
            let who = ensure_signed(origin)?;

            // Compare with storage
            let circuit_server_metadata = <CircuitServerMetadataMap<T>>::get((
                who.clone(),
                TryInto::<BoundedVec<u8, ConstU32<64>>>::try_into(ipfs_cid).unwrap(),
            ));

            // convert ascii to digits
            // first step: Vec<u8> to str; that way we can then use "to_digit"
            let input_digits_str =
                sp_std::str::from_utf8(&input_digits).expect("input_digits utf8");
            let input_digits_int: Vec<u8> = input_digits_str
                .chars()
                .map(|c| u8::try_from(c.to_digit(10u32).unwrap()).unwrap())
                .collect();

            // TODO remove the key from the map; we DO NOT want to allow retrying
            if circuit_server_metadata.0 == input_digits_int {
                Self::deposit_event(Event::TxPass { account_id: who });
                // TODO on success: call next step/callback (ie pallet-tx-XXX)
                return Ok(());
            } else {
                Self::deposit_event(Event::TxFail { account_id: who });
                return Err(Error::<T>::TxWrongInputGiven)?;
            }
        }
    }
}
