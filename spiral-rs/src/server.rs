#[cfg(target_feature = "avx2")]
use std::arch::x86_64::*;

#[cfg(target_feature = "avx2")]
use crate::aligned_memory::*;

use crate::arith::*;
use crate::gadget::*;
use crate::params::*;
use crate::poly::*;
use crate::util::*;

pub fn coefficient_expansion(
    v: &mut Vec<PolyMatrixNTT>,
    g: usize,
    stopround: usize,
    params: &Params,
    v_w_left: &Vec<PolyMatrixNTT>,
    v_w_right: &Vec<PolyMatrixNTT>,
    v_neg1: &Vec<PolyMatrixNTT>,
    max_bits_to_gen_right: usize,
) {
    let poly_len = params.poly_len;

    let mut ct = PolyMatrixRaw::zero(params, 2, 1);
    let mut ct_auto = PolyMatrixRaw::zero(params, 2, 1);
    let mut ct_auto_1 = PolyMatrixRaw::zero(params, 1, 1);
    let mut ct_auto_1_ntt = PolyMatrixNTT::zero(params, 1, 1);
    let mut ginv_ct_left = PolyMatrixRaw::zero(params, params.t_exp_left, 1);
    let mut ginv_ct_left_ntt = PolyMatrixNTT::zero(params, params.t_exp_left, 1);
    let mut ginv_ct_right = PolyMatrixRaw::zero(params, params.t_exp_right, 1);
    let mut ginv_ct_right_ntt = PolyMatrixNTT::zero(params, params.t_exp_right, 1);
    let mut w_times_ginv_ct = PolyMatrixNTT::zero(params, 2, 1);

    for r in 0..g {
        let num_in = 1 << r;
        let num_out = 2 * num_in;

        let t = (poly_len / (1 << r)) + 1;

        let neg1 = &v_neg1[r];

        for i in 0..num_out {
            if stopround > 0 && i % 2 == 1 && r > stopround
                || (r == stopround && i / 2 >= max_bits_to_gen_right)
            {
                continue;
            }

            let (w, _gadget_dim, gi_ct, gi_ct_ntt) = match i % 2 {
                0 => (
                    &v_w_left[r],
                    params.t_exp_left,
                    &mut ginv_ct_left,
                    &mut ginv_ct_left_ntt,
                ),
                1 | _ => (
                    &v_w_right[r],
                    params.t_exp_right,
                    &mut ginv_ct_right,
                    &mut ginv_ct_right_ntt,
                ),
            };

            if i < num_in {
                let (src, dest) = v.split_at_mut(num_in);
                scalar_multiply(&mut dest[i], neg1, &src[i]);
            }

            from_ntt(&mut ct, &v[i]);
            automorph(&mut ct_auto, &ct, t);
            gadget_invert_rdim(gi_ct, &ct_auto, 1);
            to_ntt_no_reduce(gi_ct_ntt, &gi_ct);
            ct_auto_1
                .data
                .as_mut_slice()
                .copy_from_slice(ct_auto.get_poly(1, 0));
            to_ntt(&mut ct_auto_1_ntt, &ct_auto_1);
            multiply(&mut w_times_ginv_ct, w, &gi_ct_ntt);

            let mut idx = 0;
            for j in 0..2 {
                for n in 0..params.crt_count {
                    for z in 0..poly_len {
                        let sum = v[i].data[idx]
                            + w_times_ginv_ct.data[idx]
                            + j * ct_auto_1_ntt.data[n * poly_len + z];
                        v[i].data[idx] = barrett_coeff_u64(params, sum, n);
                        idx += 1;
                    }
                }
            }
        }
    }
}

pub fn regev_to_gsw<'a>(
    v_gsw: &mut Vec<PolyMatrixNTT<'a>>,
    v_inp: &Vec<PolyMatrixNTT<'a>>,
    v: &PolyMatrixNTT<'a>,
    params: &'a Params,
    idx_factor: usize,
    idx_offset: usize,
) {
    assert!(v.rows == 2);
    assert!(v.cols == 2 * params.t_conv);

    let mut ginv_c_inp = PolyMatrixRaw::zero(params, 2 * params.t_conv, 1);
    let mut ginv_c_inp_ntt = PolyMatrixNTT::zero(params, 2 * params.t_conv, 1);
    let mut tmp_ct_raw = PolyMatrixRaw::zero(params, 2, 1);
    let mut tmp_ct = PolyMatrixNTT::zero(params, 2, 1);

    for i in 0..params.db_dim_2 {
        let ct = &mut v_gsw[i];
        for j in 0..params.t_gsw {
            let idx_ct = i * params.t_gsw + j;
            let idx_inp = idx_factor * (idx_ct) + idx_offset;
            ct.copy_into(&v_inp[idx_inp], 0, 2 * j + 1);
            from_ntt(&mut tmp_ct_raw, &v_inp[idx_inp]);
            gadget_invert(&mut ginv_c_inp, &tmp_ct_raw);
            to_ntt(&mut ginv_c_inp_ntt, &ginv_c_inp);
            multiply(&mut tmp_ct, v, &ginv_c_inp_ntt);
            ct.copy_into(&tmp_ct, 0, 2 * j);
        }
    }
}

pub const MAX_SUMMED: usize = 1 << 6;
pub const PACKED_OFFSET_2: i32 = 32;

#[cfg(target_feature = "avx2")]
pub fn multiply_reg_by_database(
    out: &mut Vec<PolyMatrixNTT>,
    db: &[u64],
    v_firstdim: &[u64],
    params: &Params,
    dim0: usize,
    num_per: usize,
) {
    let ct_rows = 2;
    let ct_cols = 1;
    let pt_rows = 1;
    let pt_cols = 1;

    assert!(dim0 * ct_rows >= MAX_SUMMED);

    let mut sums_out_n0_u64 = AlignedMemory64::new(4);
    let mut sums_out_n2_u64 = AlignedMemory64::new(4);

    for z in 0..params.poly_len {
        let idx_a_base = z * (ct_cols * dim0 * ct_rows);
        let mut idx_b_base = z * (num_per * pt_cols * dim0 * pt_rows);

        for i in 0..num_per {
            for c in 0..pt_cols {
                let inner_limit = MAX_SUMMED;
                let outer_limit = dim0 * ct_rows / inner_limit;

                let mut sums_out_n0_u64_acc = [0u64, 0, 0, 0];
                let mut sums_out_n2_u64_acc = [0u64, 0, 0, 0];

                for o_jm in 0..outer_limit {
                    unsafe {
                        let mut sums_out_n0 = _mm256_setzero_si256();
                        let mut sums_out_n2 = _mm256_setzero_si256();

                        for i_jm in 0..inner_limit / 4 {
                            let jm = o_jm * inner_limit + (4 * i_jm);

                            let b_inp_1 = *db.get_unchecked(idx_b_base) as i64;
                            idx_b_base += 1;
                            let b_inp_2 = *db.get_unchecked(idx_b_base) as i64;
                            idx_b_base += 1;
                            let b = _mm256_set_epi64x(b_inp_2, b_inp_2, b_inp_1, b_inp_1);

                            let v_a = v_firstdim.get_unchecked(idx_a_base + jm) as *const u64;

                            let a = _mm256_load_si256(v_a as *const __m256i);
                            let a_lo = a;
                            let a_hi_hi = _mm256_srli_epi64(a, PACKED_OFFSET_2);
                            let b_lo = b;
                            let b_hi_hi = _mm256_srli_epi64(b, PACKED_OFFSET_2);

                            sums_out_n0 =
                                _mm256_add_epi64(sums_out_n0, _mm256_mul_epu32(a_lo, b_lo));
                            sums_out_n2 =
                                _mm256_add_epi64(sums_out_n2, _mm256_mul_epu32(a_hi_hi, b_hi_hi));
                        }

                        // reduce here, otherwise we will overflow

                        _mm256_store_si256(
                            sums_out_n0_u64.as_mut_ptr() as *mut __m256i,
                            sums_out_n0,
                        );
                        _mm256_store_si256(
                            sums_out_n2_u64.as_mut_ptr() as *mut __m256i,
                            sums_out_n2,
                        );

                        for idx in 0..4 {
                            let val = sums_out_n0_u64[idx];
                            sums_out_n0_u64_acc[idx] = barrett_coeff_u64(params, val + sums_out_n0_u64_acc[idx], 0);
                        }
                        for idx in 0..4 {
                            let val = sums_out_n2_u64[idx];
                            sums_out_n2_u64_acc[idx] = barrett_coeff_u64(params, val + sums_out_n2_u64_acc[idx], 1);
                        }
                    }
                }

                for idx in 0..4 {
                    sums_out_n0_u64_acc[idx] = barrett_coeff_u64(params, sums_out_n0_u64_acc[idx], 0);
                    sums_out_n2_u64_acc[idx] = barrett_coeff_u64(params, sums_out_n2_u64_acc[idx], 1);
                }

                // output n0
                let (crt_count, poly_len) = (params.crt_count, params.poly_len);
                let mut n = 0;
                let mut idx_c = c * (crt_count * poly_len) + n * (poly_len) + z;
                out[i].data[idx_c] =
                    barrett_coeff_u64(params, sums_out_n0_u64_acc[0] + sums_out_n0_u64_acc[2], 0);
                idx_c += pt_cols * crt_count * poly_len;
                out[i].data[idx_c] =
                    barrett_coeff_u64(params, sums_out_n0_u64_acc[1] + sums_out_n0_u64_acc[3], 0);

                // output n1
                n = 1;
                idx_c = c * (crt_count * poly_len) + n * (poly_len) + z;
                out[i].data[idx_c] =
                    barrett_coeff_u64(params, sums_out_n2_u64_acc[0] + sums_out_n2_u64_acc[2], 1);
                idx_c += pt_cols * crt_count * poly_len;
                out[i].data[idx_c] =
                    barrett_coeff_u64(params, sums_out_n2_u64_acc[1] + sums_out_n2_u64_acc[3], 1);
            }
        }
    }
}

pub fn generate_random_db_and_get_item<'a>(
    params: &'a Params,
    item_idx: usize,
) -> (PolyMatrixRaw<'a>, Vec<u64>) {
    let mut rng = get_seeded_rng();

    let trials = params.n * params.n;
    let dim0 = 1 << params.db_dim_1;
    let num_per = 1 << params.db_dim_2;
    let num_items = dim0 * num_per;
    let db_size_words = trials * num_items * params.poly_len;
    let mut v = vec![0u64; db_size_words];

    let mut item = PolyMatrixRaw::zero(params, params.n, params.n);

    for trial in 0..trials {
        for i in 0..num_items {
            let ii = i % num_per;
            let j = i / num_per;

            let mut db_item = PolyMatrixRaw::random_rng(params, 1, 1, &mut rng);
            db_item.reduce_mod(params.pt_modulus);
            
            if i == item_idx {
                item.copy_into(&db_item, trial / params.n, trial % params.n);
            }

            for z in 0..params.poly_len {
                db_item.data[z] = recenter_mod(db_item.data[z], params.pt_modulus, params.modulus);
            }

            let db_item_ntt = db_item.ntt();
            for z in 0..params.poly_len {
                let idx_dst = calc_index(
                    &[trial, z, ii, j],
                    &[trials, params.poly_len, num_per, dim0],
                );

                v[idx_dst] = db_item_ntt.data[z]
                    | (db_item_ntt.data[params.poly_len + z] << PACKED_OFFSET_2);
            }
        }
    }
    (item, v)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{client::*};
    use rand::{prelude::StdRng, Rng};

    fn get_params() -> Params {
        let mut params = get_expansion_testing_params();
        params.db_dim_1 = 6;
        params.db_dim_2 = 2;
        params.t_exp_right = 8;
        params
    }

    fn dec_reg<'a>(
        params: &'a Params,
        ct: &PolyMatrixNTT<'a>,
        client: &mut Client<'a, StdRng>,
        scale_k: u64,
    ) -> u64 {
        let dec = client.decrypt_matrix_reg(ct).raw();
        let mut val = dec.data[0] as i64;
        if val >= (params.modulus / 2) as i64 {
            val -= params.modulus as i64;
        }
        let val_rounded = f64::round(val as f64 / scale_k as f64) as i64;
        if val_rounded == 0 {
            0
        } else {
            1
        }
    }

    fn dec_gsw<'a>(
        params: &'a Params,
        ct: &PolyMatrixNTT<'a>,
        client: &mut Client<'a, StdRng>,
    ) -> u64 {
        let dec = client.decrypt_matrix_reg(ct).raw();
        let idx = (params.t_gsw - 1) * params.poly_len + params.poly_len; // this offset should encode a large value
        let mut val = dec.data[idx] as i64;
        if val >= (params.modulus / 2) as i64 {
            val -= params.modulus as i64;
        }
        if val < 100 {
            0
        } else {
            1
        }
    }

    #[test]
    fn coefficient_expansion_is_correct() {
        let params = get_params();
        let v_neg1 = params.get_v_neg1();
        let mut seeded_rng = get_seeded_rng();
        let mut client = Client::init(&params, &mut seeded_rng);
        let public_params = client.generate_keys();

        let mut v = Vec::new();
        for _ in 0..(1 << (params.db_dim_1 + 1)) {
            v.push(PolyMatrixNTT::zero(&params, 2, 1));
        }

        let target = 7;
        let scale_k = params.modulus / params.pt_modulus;
        let mut sigma = PolyMatrixRaw::zero(&params, 1, 1);
        sigma.data[target] = scale_k;
        v[0] = client.encrypt_matrix_reg(&sigma.ntt());
        let test_ct = client.encrypt_matrix_reg(&sigma.ntt());

        let v_w_left = public_params.v_expansion_left.unwrap();
        let v_w_right = public_params.v_expansion_right.unwrap();
        coefficient_expansion(
            &mut v,
            client.g,
            client.stop_round,
            &params,
            &v_w_left,
            &v_w_right,
            &v_neg1,
            params.t_gsw * params.db_dim_2,
        );

        assert_eq!(dec_reg(&params, &test_ct, &mut client, scale_k), 0);

        for i in 0..v.len() {
            if i == target {
                assert_eq!(dec_reg(&params, &v[i], &mut client, scale_k), 1);
            } else {
                assert_eq!(dec_reg(&params, &v[i], &mut client, scale_k), 0);
            }
        }
    }

    #[test]
    fn regev_to_gsw_is_correct() {
        let mut params = get_params();
        params.db_dim_2 = 1;
        let mut seeded_rng = get_seeded_rng();
        let mut client = Client::init(&params, &mut seeded_rng);
        let public_params = client.generate_keys();

        let mut enc_constant = |val| {
            let mut sigma = PolyMatrixRaw::zero(&params, 1, 1);
            sigma.data[0] = val;
            client.encrypt_matrix_reg(&sigma.ntt())
        };

        let v = &public_params.v_conversion.unwrap()[0];

        let bits_per = get_bits_per(&params, params.t_gsw);
        let mut v_inp_1 = Vec::new();
        let mut v_inp_0 = Vec::new();
        for i in 0..params.t_gsw {
            let val = 1u64 << (bits_per * i);
            v_inp_1.push(enc_constant(val));
            v_inp_0.push(enc_constant(0));
        }

        let mut v_gsw = Vec::new();
        v_gsw.push(PolyMatrixNTT::zero(&params, 2, 2 * params.t_gsw));

        regev_to_gsw(&mut v_gsw, &v_inp_1, v, &params, 1, 0);

        assert_eq!(dec_gsw(&params, &v_gsw[0], &mut client), 1);

        regev_to_gsw(&mut v_gsw, &v_inp_0, v, &params, 1, 0);

        assert_eq!(dec_gsw(&params, &v_gsw[0], &mut client), 0);
    }

    #[test]
    fn multiply_reg_by_database_is_correct() {
        let params = get_params();
        let mut seeded_rng = get_seeded_rng();

        let dim0 = 1 << params.db_dim_1;
        let num_per = 1 << params.db_dim_2;
        let scale_k = params.modulus / params.pt_modulus;

        let target_idx = seeded_rng.gen::<usize>() % (dim0 * num_per);
        let target_idx_dim0 = target_idx / num_per;
        let target_idx_num_per = target_idx % num_per;

        let mut client = Client::init(&params, &mut seeded_rng);
        _ = client.generate_keys();

        let (corr_item, db) = generate_random_db_and_get_item(&params, target_idx);

        let mut v_reg = Vec::new();
        for i in 0..dim0 {
            let val = if i == target_idx_dim0 { scale_k } else { 0 };
            let sigma = PolyMatrixRaw::single_value(&params, val).ntt();
            v_reg.push(client.encrypt_matrix_reg(&sigma));
        }

        let v_reg_sz = dim0 * 2 * params.poly_len;
        let mut v_reg_reoriented = AlignedMemory64::new(v_reg_sz);
        reorient_reg_ciphertexts(&params, v_reg_reoriented.as_mut_slice(), &v_reg);

        let mut out = Vec::with_capacity(num_per);
        for _ in 0..dim0 {
            out.push(PolyMatrixNTT::zero(&params, 2, 1));
        }
        multiply_reg_by_database(&mut out, db.as_slice(), v_reg_reoriented.as_slice(), &params, dim0, num_per);

        // decrypt
        let dec = client.decrypt_matrix_reg(&out[target_idx_num_per]).raw();
        let mut dec_rescaled = PolyMatrixRaw::zero(&params, 1, 1);
        for z in 0..params.poly_len {
            dec_rescaled.data[z] = rescale(dec.data[z], params.modulus, params.pt_modulus);
        }

        for z in 0..params.poly_len {
            // println!("{:?} {:?}", dec_rescaled.data[z], corr_item.data[z]);
            assert_eq!(dec_rescaled.data[z], corr_item.data[z]);
        }
    }
}
