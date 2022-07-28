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

    /// the maximum accepted length for "register_mobile"
    /// WARNING: MUST be big enough to include encoded version(ie PKCS etc)
    type MAX_PUB_KEY_LEN = ConstU32<256>;

    /// Easy way to make a link b/w a "message" and "pinpad" circuits
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
    pub struct MobilePackage {
        pub pub_key: BoundedVec<u8, MAX_PUB_KEY_LEN>,
    }

    #[pallet::storage]
    #[pallet::getter(fn circuit_server_metadata_map)]
    pub(super) type MobileRegistryMap<T: Config> = StorageMap<
        _,
        Twox128,
        T::AccountId,
        //  Struct containing both message_digits and pinpad_digits
        MobilePackage,
        // TODO?
        // ValueQuery,
    >;

    #[pallet::pallet]
    #[pallet::generate_store(pub(super) trait Store)]
    pub struct Pallet<T>(_);

    // Pallets use events to inform users when important changes are made.
    // https://docs.substrate.io/v3/runtime/events-and-errors
    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        NewMobileRegistered { account_id: T::AccountId },
    }

    // Errors inform users that something went wrong.
    #[pallet::error]
    pub enum Error<T> {
        /// register_mobile: pub_key SHOULD be at least 32 bytes(for ECC)
        // TODO support 2048(for RSA)?
        InvalidKeySize,
    }

    impl<T: Config> Pallet<T> {
        /// Check the pub_key is at least 32 bytes in length
        pub fn ensure_pub_key_valid(pub_key: &Vec<u8>) -> Result<(), Error<T>> {
            match pub_key.len() {
                32.. => Ok(()),
                _ => Err(Error::<T>::InvalidKeySize),
            }
        }
    }

    // Dispatchable functions allows users to interact with the pallet and invoke state changes.
    // These functions materialize as "extrinsics", which are often compared to transactions.
    // Dispatchable functions must be annotated with a weight and must return a DispatchResult.
    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::weight(10_000 + T::DbWeight::get().writes(1))]
        pub fn register_mobile(origin: OriginFor<T>, pub_key: Vec<u8>) -> DispatchResult {
            // Check that the extrinsic was signed and get the signer.
            // This function will return an error if the extrinsic is not signed.
            // https://docs.substrate.io/v3/runtime/origins
            let who = ensure_signed(origin)?;

            Self::ensure_pub_key_valid(&pub_key)?;

            crate::Pallet::<T>::deposit_event(Event::NewMobileRegistered {
                account_id: who.clone(),
            });

            // Update storage.
            <MobileRegistryMap<T>>::insert(
                who,
                MobilePackage {
                    pub_key: TryInto::<BoundedVec<u8, MAX_PUB_KEY_LEN>>::try_into(pub_key).unwrap(),
                },
            );

            Ok(())
        }
    }
}
