#![allow(non_snake_case)]
/*
    sin-city

    Copyright 2018 by Kzen Networks

    This file is part of paradise-city library
    (https://github.com/KZen-networks/sin-city)

    sin-city is free software: you can redistribute
    it and/or modify it under the terms of the GNU General Public
    License as published by the Free Software Foundation, either
    version 3 of the License, or (at your option) any later version.

    @license GPL-3.0+ <https://github.com/KZen-networks/sin-city/blob/master/LICENSE>
*/
const SECURITY_BITS: usize = 256;

extern crate curv;
extern crate itertools;
extern crate multi_party_ecdsa;
extern crate paillier;

mod tests;

use curv::arithmetic::traits::Modulo;
use curv::arithmetic::traits::Samplable;
use curv::cryptographic_primitives::commitments::hash_commitment::HashCommitment;
use curv::cryptographic_primitives::commitments::traits::Commitment;
use curv::cryptographic_primitives::hashing::hash_sha256::HSha256;
use curv::cryptographic_primitives::hashing::traits::Hash;
use curv::cryptographic_primitives::proofs::sigma_dlog::*;
use curv::cryptographic_primitives::proofs::sigma_ec_ddh::NISigmaProof;
use curv::cryptographic_primitives::proofs::sigma_ec_ddh::{
    ECDDHProof, ECDDHStatement, ECDDHWitness,
};
use curv::elliptic::curves::traits::{ECPoint, ECScalar};
use curv::{BigInt, FE, GE};
use itertools::iterate;
use paillier::Paillier;
use paillier::{Add, Decrypt, Encrypt, KeyGeneration, Mul};
use paillier::{DecryptionKey, EncryptionKey, RawCiphertext, RawPlaintext};

pub struct MultiHopLock {
    pub num_parties: usize,
    pub y_0: FE,
    pub setup_chain: Vec<ChainLink>,
    pub setup_chain_link_u_n: ChainLinkUn,
}

pub struct ChainLink {
    pub Y_i_minus_1: GE,
    pub Y_i: GE,
    pub y_i: FE,
    pub proof: DLogProof,
}

pub struct ChainLinkUn {
    pub Y_i_minus_1: GE,
    pub k_n: FE,
}

pub struct LockParty0Message1 {
    ddh_proof: ECDDHProof,
    R_0: GE,
    R_0_tag: GE,
}

pub struct LockParty1Message1 {
    comm: BigInt,
}

pub struct DecommitLockParty1Message1 {
    blind_factor: BigInt,
    R_1: GE,
    R_1_tag: GE,
    ddh_proof: ECDDHProof,
}

pub struct PartialSig {
    pub c_tag: BigInt,
}

pub struct LockParty1Message2 {
    decomm: DecommitLockParty1Message1,
    partial_sig: PartialSig,
}

pub struct LockParty0Message2 {
    s_tag: FE,
}

#[derive(Debug)]
pub struct SL {
    w_0: FE,
    w_1: FE,
    pk: GE,
}

#[derive(Debug)]
pub struct SR {
    s_tag: FE,
    message: FE,
}
use std::fmt;

#[derive(Debug)]
pub struct K {
    r: FE,
    s: FE,
}

pub struct L {
    m: FE,
    pk: GE,
}

pub struct Release {} // TODO: consider adding session id

impl MultiHopLock {
    pub fn setup(n: usize) -> MultiHopLock {
        let y_0: FE = ECScalar::new_random();
        let g: GE = ECPoint::generator();
        // let Y_0 = g * &y_0;
        let y_i_vec = (0..n).map(|_| ECScalar::new_random()).collect::<Vec<FE>>();

        let tuple_y_i_cumsum_index =
            iterate((y_0, 0 as usize), |y_i| (y_i.0 + y_i_vec[y_i.1], y_i.1 + 1))
                .take(n)
                .collect::<Vec<(FE, usize)>>();
        let y_i_cumsum = tuple_y_i_cumsum_index
            .iter()
            .map(|i| i.0)
            .collect::<Vec<FE>>();

        let tuple_Y_i_vec_proof_vec = (0..n)
            .map(|i| (g * y_i_cumsum[i], DLogProof::prove(&y_i_cumsum[i])))
            .collect::<Vec<(GE, DLogProof)>>();

        let chain_link_vec = (1..n)
            .map(|i| ChainLink {
                Y_i_minus_1: tuple_Y_i_vec_proof_vec[i - 1].0.clone(),
                Y_i: tuple_Y_i_vec_proof_vec[i].0.clone(),
                y_i: y_i_vec[i - 1].clone(),
                proof: tuple_Y_i_vec_proof_vec[i].1.clone(),
            })
            .collect::<Vec<ChainLink>>();
        let chain_link_u_n = ChainLinkUn {
            Y_i_minus_1: tuple_Y_i_vec_proof_vec[n - 1].0.clone(),
            k_n: y_i_cumsum[n - 1].clone(),
        };

        return MultiHopLock {
            num_parties: n,
            y_0,
            setup_chain: chain_link_vec,
            setup_chain_link_u_n: chain_link_u_n,
        };
    }

    pub fn verify_setup(chain_i: &ChainLink) -> Result<(), ()> {
        let verified = DLogProof::verify(&chain_i.proof);
        let g: GE = ECPoint::generator();
        let y_i_G = g * &chain_i.y_i;

        match verified {
            Err(_) => Err(()),
            Ok(_) => {
                if chain_i.proof.pk == chain_i.Y_i && y_i_G + chain_i.Y_i_minus_1 == chain_i.Y_i {
                    Ok(())
                } else {
                    Err(())
                }
            }
        }
    }
}

impl LockParty1Message1 {
    pub fn first_message(Y_1_tag: &GE) -> (FE, DecommitLockParty1Message1, LockParty1Message1) {
        let g: GE = ECPoint::generator();

        let r_1: FE = ECScalar::new_random();
        let R_1 = g * &r_1;

        let w = ECDDHWitness { x: r_1.clone() };
        let R_1_tag = Y_1_tag * &r_1;
        let delta = ECDDHStatement {
            g1: g.clone(),
            h1: R_1.clone(),
            g2: Y_1_tag.clone(),
            h2: R_1_tag.clone(),
        };
        let ddh_proof = ECDDHProof::prove(&w, &delta);

        // we use hash based commitment
        let pk_commitment_blind_factor = BigInt::sample(SECURITY_BITS);
        // hash R1, R1_tag
        let commit_hashed_message =
            HSha256::create_hash_from_ge(&[&R_1, &R_1_tag, &ddh_proof.a1, &ddh_proof.a2]);

        let commitment = HashCommitment::create_commitment_with_user_defined_randomness(
            &commit_hashed_message.to_big_int(),
            &pk_commitment_blind_factor,
        );
        let decommit = DecommitLockParty1Message1 {
            blind_factor: pk_commitment_blind_factor,
            R_1,
            R_1_tag,
            ddh_proof,
        };
        (r_1, decommit, LockParty1Message1 { comm: commitment })
    }
}

impl LockParty0Message1 {
    pub fn first_message(Y_0: &GE) -> (FE, LockParty0Message1) {
        let g: GE = ECPoint::generator();
        let r_0: FE = ECScalar::new_random();
        let R_0 = &g * &r_0;
        let w = ECDDHWitness { x: r_0.clone() };
        let R_0_tag = Y_0 * &r_0;
        let delta = ECDDHStatement {
            g1: g.clone(),
            h1: R_0.clone(),
            g2: Y_0.clone(),
            h2: R_0_tag.clone(),
        };
        let ddh_proof = ECDDHProof::prove(&w, &delta);
        (
            r_0,
            LockParty0Message1 {
                ddh_proof,
                R_0,
                R_0_tag,
            },
        )
    }
}

impl LockParty1Message2 {
    pub fn second_message(
        lock_party0_message1: &LockParty0Message1,
        decom_message1: DecommitLockParty1Message1,
        ek: &EncryptionKey,
        x_1: &FE,
        encrypted_secret_share: &BigInt,
        message: &BigInt,
        r_1: &FE,
        Y_i_minus_1: &GE,
    ) -> LockParty1Message2 {
        // verify counter party NIZK:
        let g: GE = ECPoint::generator();
        let delta = ECDDHStatement {
            g1: g,
            h1: lock_party0_message1.R_0.clone(),
            g2: Y_i_minus_1.clone(),
            h2: lock_party0_message1.R_0_tag.clone(),
        };
        ECDDHProof::verify(&lock_party0_message1.ddh_proof, &delta).expect("bad NIZK");
        // R
        let R = lock_party0_message1.R_0_tag * r_1;
        let q = FE::q();

        let rx = R.x_coor().unwrap().mod_floor(&q);
        let rho = BigInt::sample_below(&q.pow(2));
        let r_1_inv = r_1.invert();
        let partial_sig = rho * &q + BigInt::mod_mul(&r_1_inv.to_big_int(), message, &q);
        let c1 = Paillier::encrypt(ek, RawPlaintext::from(partial_sig));
        let v = BigInt::mod_mul(
            &r_1_inv.to_big_int(),
            &BigInt::mod_mul(&rx, &x_1.to_big_int(), &q),
            &q,
        );
        let c2 = Paillier::mul(
            ek,
            RawCiphertext::from(encrypted_secret_share.clone()),
            RawPlaintext::from(v),
        );

        let partial_sig = PartialSig {
            c_tag: Paillier::add(ek, c2, c1).0.into_owned(),
        };

        LockParty1Message2 {
            decomm: decom_message1,
            partial_sig,
        }
    }
}

impl LockParty0Message2 {
    pub fn second_message(
        dk: &DecryptionKey,
        lock_party1_message2: LockParty1Message2,
        lock_party1_message1: LockParty1Message1,
        message: &BigInt,
        r_0: FE,
        Y_i: &GE,
        pubkey: &GE,
    ) -> (FE, LockParty0Message2) {
        // verify commitment:
        let commit_hashed_message = HSha256::create_hash_from_ge(&[
            &lock_party1_message2.decomm.R_1,
            &lock_party1_message2.decomm.R_1_tag,
            &lock_party1_message2.decomm.ddh_proof.a1,
            &lock_party1_message2.decomm.ddh_proof.a2,
        ]);
        let commitment = HashCommitment::create_commitment_with_user_defined_randomness(
            &commit_hashed_message.to_big_int(),
            &lock_party1_message2.decomm.blind_factor,
        );
        assert_eq!(commitment, lock_party1_message1.comm);
        // verify counter party NIZK:
        let g: GE = ECPoint::generator();
        let delta = ECDDHStatement {
            g1: g,
            h1: lock_party1_message2.decomm.R_1.clone(),
            g2: Y_i.clone(),
            h2: lock_party1_message2.decomm.R_1_tag.clone(),
        };
        ECDDHProof::verify(&lock_party1_message2.decomm.ddh_proof, &delta).expect("bad NIZK");
        // R
        let R = lock_party1_message2.decomm.R_1_tag * r_0;

        let r_x = R.x_coor().unwrap().mod_floor(&FE::q());
        let r_0_inv = r_0.invert();

        let s = Paillier::decrypt(
            dk,
            &RawCiphertext::from(lock_party1_message2.partial_sig.c_tag),
        );
        let s_fe: FE = ECScalar::from(&s.0);
        let s_R_1 = lock_party1_message2.decomm.R_1 * s_fe;
        let r_x_fe: FE = ECScalar::from(&r_x);
        let r_x_pk = pubkey * &r_x_fe;
        let e_fe: FE = ECScalar::from(message);
        let e_g = g * e_fe;
        assert_eq!(s_R_1, e_g + r_x_pk);
        let s_tag = s_fe.mul(&r_0_inv.get_element());
        (s_tag, LockParty0Message2 { s_tag })
    }

    pub fn verify(
        &self,
        lock_party0_message1: LockParty0Message1,
        r_1: &FE,
        pubkey: &GE,
        message: &BigInt,
    ) -> (FE, FE) {
        let g: GE = ECPoint::generator();
        let R = lock_party0_message1.R_0_tag * r_1;
        let q = FE::q();
        let r_x = R.x_coor().unwrap().mod_floor(&q);
        let r_x_fe: FE = ECScalar::from(&r_x);
        let r_x_pk = pubkey * &r_x_fe;
        let s_tag_r_1_R_0 = lock_party0_message1.R_0 * r_1 * self.s_tag;
        let e_fe: FE = ECScalar::from(message);
        let e_g = g * e_fe;

        assert_eq!(s_tag_r_1_R_0, e_g + r_x_pk);
        (self.s_tag.clone(), r_x_fe)
    }
}

pub fn get_paillier_keys() -> (EncryptionKey, DecryptionKey) {
    Paillier::keypair().keys()
}

impl Release {
    pub fn release_i(chain_i: &ChainLink, k_i_plus_1: K, s_L: &SL, s_R: &SR) -> Result<K, ()> {
        let l = L {
            m: s_R.message,
            pk: s_L.pk,
        };
        let q = FE::q();
        let s_inv = k_i_plus_1.s.invert();
        let s_tag_div_s_inv = s_inv * s_R.s_tag;
        let s_tag_div_s_inv_minus_y = s_tag_div_s_inv.sub(&chain_i.y_i.get_element());
        let s_tag_div_s_inv_minus_y_inv = s_tag_div_s_inv_minus_y.invert();
        let t = s_L.w_1 * s_tag_div_s_inv_minus_y_inv;
        let t_bn = t.to_big_int();
        let t_bn_neg = q.clone() - t_bn.clone();
        let t_min_bn = BigInt::min(t_bn, t_bn_neg);
        let t_min = ECScalar::from(&t_min_bn);

        let k = K {
            r: s_L.w_0.clone(),
            s: t_min,
        };
        if vf(&l, &k).is_ok() {
            return Ok(K {
                r: FE::zero(),
                s: k.s.clone(),
            });
        }

        let s_tag_div_s_inv_bn = s_tag_div_s_inv.to_big_int();
        let s_tag_div_s_inv_bn_neg = q.clone() - s_tag_div_s_inv_bn;
        let s_tag_div_s_inv_neg: FE = ECScalar::from(&s_tag_div_s_inv_bn_neg);
        let s_tag_div_s_inv_minus_y_neg = s_tag_div_s_inv_neg.sub(&chain_i.y_i.get_element());
        let s_tag_div_s_inv_minus_y_inv_neg = s_tag_div_s_inv_minus_y_neg.invert();
        let t_tag = s_L.w_1 * s_tag_div_s_inv_minus_y_inv_neg;
        let t_tag_bn = t_tag.to_big_int();
        let t_tag_bn_neg = q - t_tag_bn.clone();
        let t_tag_min_bn = BigInt::min(t_tag_bn, t_tag_bn_neg);
        let t_tag_min: FE = ECScalar::from(&t_tag_min_bn);

        let k = K {
            r: s_L.w_0.clone(),
            s: t_tag_min,
        };
        match vf(&l, &k).is_ok() {
            true => Ok(K {
                r: FE::zero(),
                s: k.s.clone(),
            }),
            false => Err(()),
        }
    }

    pub fn release_n_minus_1(
        chain_n_minus_1: &ChainLink,
        chain_n: &ChainLinkUn,
        s_L_n: &SL,
        s_L_n_minus_1: &SL,
        s_R: &SR,
    ) -> Result<K, ()> {
        let l = L {
            m: s_R.message,
            pk: s_L_n_minus_1.pk,
        };
        let q = FE::q();
        let s = chain_n.k_n.invert() * s_L_n.w_1;
        let s_inv = s.invert();
        let s_tag_div_s_inv = s_inv * s_R.s_tag;
        let s_tag_div_s_inv_minus_y = s_tag_div_s_inv.sub(&chain_n_minus_1.y_i.get_element());
        let s_tag_div_s_inv_minus_y_inv = s_tag_div_s_inv_minus_y.invert();
        let t = s_L_n_minus_1.w_1 * s_tag_div_s_inv_minus_y_inv;
        let t_bn = t.to_big_int();
        let t_bn_neg = q.clone() - t_bn.clone();
        let t_min_bn = BigInt::min(t_bn.clone(), t_bn_neg.clone());
        //  let t_min_be = t_bn_neg;
        let t_min = ECScalar::from(&t_min_bn);

        let k = K {
            r: s_L_n.w_0.clone(),
            s: s.clone(),
        };
        vf(&l, &k).is_ok();

        let k = K {
            r: s_L_n_minus_1.w_0.clone(),
            s: t_min,
        };
        if vf(&l, &k).is_ok() {
            return Ok(K {
                r: FE::zero(),
                s: k.s.clone(),
            });
        }

        let s_tag_div_s_inv_bn = s_tag_div_s_inv.to_big_int();
        let s_tag_div_s_inv_bn_neg = q.clone() - s_tag_div_s_inv_bn;
        let s_tag_div_s_inv_neg: FE = ECScalar::from(&s_tag_div_s_inv_bn_neg);
        let s_tag_div_s_inv_minus_y_neg =
            s_tag_div_s_inv_neg.sub(&chain_n_minus_1.y_i.get_element());
        let s_tag_div_s_inv_minus_y_inv_neg = s_tag_div_s_inv_minus_y_neg.invert();
        let t_tag = s_L_n_minus_1.w_1 * s_tag_div_s_inv_minus_y_inv_neg;
        let t_tag_bn = t_tag.to_big_int();
        let t_tag_bn_neg = q - t_tag_bn.clone();
        let t_tag_min_bn = BigInt::min(t_tag_bn, t_tag_bn_neg);
        let t_tag_min: FE = ECScalar::from(&t_tag_min_bn);

        let k = K {
            r: s_L_n_minus_1.w_0.clone(),
            s: t_tag_min,
        };
        match vf(&l, &k).is_ok() {
            true => Ok(K {
                r: FE::zero(),
                s: k.s.clone(),
            }),
            false => Err(()),
        }
    }
}

pub fn vf(l: &L, k: &K) -> Result<(), ()> {
    let q = FE::q();
    let q_half = (q - BigInt::one()) / BigInt::from(2);
    let g: GE = ECPoint::generator();
    let s_inv = k.s.invert();
    let m_s_inv_g = g * l.m * s_inv;
    let r_s_inv_pk = l.pk * k.r * s_inv;
    let rhs = m_s_inv_g + r_s_inv_pk;
    if k.r.to_big_int() == rhs.x_coor().unwrap() && k.s.to_big_int() < q_half {
        Ok(())
    } else {
        Err(())
    }
}
