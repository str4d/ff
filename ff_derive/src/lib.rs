#![recursion_limit="1024"]

extern crate proc_macro;
extern crate syn;
#[macro_use]
extern crate quote;

extern crate num_bigint;
extern crate num_traits;
extern crate num_integer;

use num_integer::Integer;
use num_traits::{Zero, One, ToPrimitive};
use num_bigint::BigUint;
use std::str::FromStr;

#[proc_macro_derive(PrimeField, attributes(PrimeFieldModulus, PrimeFieldGenerator))]
pub fn prime_field(
    input: proc_macro::TokenStream
) -> proc_macro::TokenStream
{
    // Construct a string representation of the type definition
    let s = input.to_string();
    
    // Parse the string representation
    let ast = syn::parse_derive_input(&s).unwrap();

    // The struct we're deriving for is a wrapper around a "Repr" type we must construct.
    let repr_ident = fetch_wrapped_ident(&ast.body)
                     .expect("PrimeField derive only operates over tuple structs of a single item");

    // We're given the modulus p of the prime field
    let modulus: BigUint = fetch_attr("PrimeFieldModulus", &ast.attrs)
                           .expect("Please supply a PrimeFieldModulus attribute")
                           .parse().expect("PrimeFieldModulus should be a number");

    // We may be provided with a generator of p - 1 order. It is required that this generator be quadratic
    // nonresidue.
    let generator: BigUint = fetch_attr("PrimeFieldGenerator", &ast.attrs)
                             .expect("Please supply a PrimeFieldGenerator attribute")
                             .parse().expect("PrimeFieldGenerator should be a number");

    // The arithmetic in this library only works if the modulus*2 is smaller than the backing
    // representation. Compute the number of limbs we need.
    let mut limbs = 1;
    {
        let mod2 = (&modulus) << 1; // modulus * 2
        let mut cur = BigUint::one() << 64; // always 64-bit limbs for now
        while cur < mod2 {
            limbs += 1;
            cur = cur << 64;
        }
    }

    let mut gen = quote::Tokens::new();

    gen.append(prime_field_repr_impl(&repr_ident, limbs));
    gen.append(prime_field_constants_and_sqrt(&ast.ident, &repr_ident, modulus, limbs, generator));
    gen.append(prime_field_impl(&ast.ident, &repr_ident, limbs));
    
    // Return the generated impl
    gen.parse().unwrap()
}

/// Fetches the ident being wrapped by the type we're deriving.
fn fetch_wrapped_ident(
    body: &syn::Body
) -> Option<syn::Ident>
{
    match body {
        &syn::Body::Struct(ref variant_data) => {
            let fields = variant_data.fields();
            if fields.len() == 1 {
                match fields[0].ty {
                    syn::Ty::Path(_, ref path) => {
                        if path.segments.len() == 1 {
                            return Some(path.segments[0].ident.clone());
                        }
                    },
                    _ => {}
                }
            }
        },
        _ => {}
    };

    None
}

/// Fetch an attribute string from the derived struct.
fn fetch_attr(
    name: &str,
    attrs: &[syn::Attribute]
) -> Option<String>
{
    for attr in attrs {
        if attr.name() == name {
            match attr.value {
                syn::MetaItem::NameValue(_, ref val) => {
                    match val {
                        &syn::Lit::Str(ref s, _) => {
                            return Some(s.clone())
                        },
                        _ => {
                            panic!("attribute {} should be a string", name);
                        }
                    }
                },
                _ => {
                    panic!("attribute {} should be a string", name);
                }
            }
        }
    }

    None
}

// Implement PrimeFieldRepr for the wrapped ident `repr` with `limbs` limbs.
fn prime_field_repr_impl(
    repr: &syn::Ident,
    limbs: usize
) -> quote::Tokens
{
    quote! {
        #[derive(Copy, Clone, PartialEq, Eq, Default)]
        pub struct #repr(pub [u64; #limbs]);

        impl ::rand::Rand for #repr {
            #[inline(always)]
            fn rand<R: ::rand::Rng>(rng: &mut R) -> Self {
                #repr(rng.gen())
            }
        }

        impl ::std::fmt::Debug for #repr
        {
            fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                try!(write!(f, "0x"));
                for i in self.0.iter().rev() {
                    try!(write!(f, "{:016x}", *i));
                }

                Ok(())
            }
        }

        impl AsRef<[u64]> for #repr {
            #[inline(always)]
            fn as_ref(&self) -> &[u64] {
                &self.0
            }
        }

        impl From<u64> for #repr {
            #[inline(always)]
            fn from(val: u64) -> #repr {
                use std::default::Default;

                let mut repr = Self::default();
                repr.0[0] = val;
                repr
            }
        }

        impl Ord for #repr {
            #[inline(always)]
            fn cmp(&self, other: &#repr) -> ::std::cmp::Ordering {
                for (a, b) in self.0.iter().rev().zip(other.0.iter().rev()) {
                    if a < b {
                        return ::std::cmp::Ordering::Less
                    } else if a > b {
                        return ::std::cmp::Ordering::Greater
                    }
                }

                ::std::cmp::Ordering::Equal
            }
        }

        impl PartialOrd for #repr {
            #[inline(always)]
            fn partial_cmp(&self, other: &#repr) -> Option<::std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }

        impl ::ff::PrimeFieldRepr for #repr {
            #[inline(always)]
            fn is_odd(&self) -> bool {
                self.0[0] & 1 == 1
            }

            #[inline(always)]
            fn is_even(&self) -> bool {
                !self.is_odd()
            }

            #[inline(always)]
            fn is_zero(&self) -> bool {
                self.0.iter().all(|&e| e == 0)
            }

            #[inline(always)]
            fn div2(&mut self) {
                let mut t = 0;
                for i in self.0.iter_mut().rev() {
                    let t2 = *i << 63;
                    *i >>= 1;
                    *i |= t;
                    t = t2;
                }
            }

            #[inline(always)]
            fn mul2(&mut self) {
                let mut last = 0;
                for i in self.0.iter_mut() {
                    let tmp = *i >> 63;
                    *i <<= 1;
                    *i |= last;
                    last = tmp;
                }
            }

            #[inline(always)]
            fn num_bits(&self) -> u32 {
                let mut ret = (#limbs as u32) * 64;
                for i in self.0.iter().rev() {
                    let leading = i.leading_zeros();
                    ret -= leading;
                    if leading != 64 {
                        break;
                    }
                }

                ret
            }

            #[inline(always)]
            fn add_nocarry(&mut self, other: &#repr) -> bool {
                let mut carry = 0;

                for (a, b) in self.0.iter_mut().zip(other.0.iter()) {
                    *a = ::ff::adc(*a, *b, &mut carry);
                }

                carry != 0
            }

            #[inline(always)]
            fn sub_noborrow(&mut self, other: &#repr) -> bool {
                let mut borrow = 0;

                for (a, b) in self.0.iter_mut().zip(other.0.iter()) {
                    *a = ::ff::sbb(*a, *b, &mut borrow);
                }

                borrow != 0
            }
        }
    }
}

/// Convert BigUint into a vector of 64-bit limbs.
fn biguint_to_u64_vec(
    mut v: BigUint,
    limbs: usize
) -> Vec<u64>
{
    let m = BigUint::one() << 64;
    let mut ret = vec![];

    while v > BigUint::zero() {
        ret.push((&v % &m).to_u64().unwrap());
        v = v >> 64;
    }

    while ret.len() < limbs {
        ret.push(0);
    }

    assert!(ret.len() == limbs);

    ret
}

fn biguint_num_bits(
    mut v: BigUint
) -> u32
{
    let mut bits = 0;

    while v != BigUint::zero() {
        v = v >> 1;
        bits += 1;
    }

    bits
}

/// BigUint modular exponentiation by square-and-multiply.
fn exp(
    base: BigUint,
    exp: &BigUint,
    modulus: &BigUint
) -> BigUint
{
    let mut ret = BigUint::one();

    for i in exp.to_bytes_be()
                .into_iter()
                .flat_map(|x| (0..8).rev().map(move |i| (x >> i).is_odd()))
    {
        ret = (&ret * &ret) % modulus;
        if i {
            ret = (ret * &base) % modulus;
        }
    }

    ret
}

#[test]
fn test_exp() {
    assert_eq!(
        exp(
            BigUint::from_str("4398572349857239485729348572983472345").unwrap(),
            &BigUint::from_str("5489673498567349856734895").unwrap(),
            &BigUint::from_str("52435875175126190479447740508185965837690552500527637822603658699938581184513").unwrap()
        ),
        BigUint::from_str("4371221214068404307866768905142520595925044802278091865033317963560480051536").unwrap()
    );
}

fn prime_field_constants_and_sqrt(
    name: &syn::Ident,
    repr: &syn::Ident,
    modulus: BigUint,
    limbs: usize,
    generator: BigUint
) -> quote::Tokens
{
    let modulus_num_bits = biguint_num_bits(modulus.clone());

    // The number of bits we should "shave" from a randomly sampled reputation, i.e.,
    // if our modulus is 381 bits and our representation is 384 bits, we should shave
    // 3 bits from the beginning of a randomly sampled 384 bit representation to 
    // reduce the cost of rejection sampling.
    let repr_shave_bits = (64 * limbs as u32) - biguint_num_bits(modulus.clone());

    // Compute R = 2**(64 * limbs) mod m
    let r = (BigUint::one() << (limbs * 64)) % &modulus;

    // modulus - 1 = 2^s * t
    let mut s: usize = 0;
    let mut t = &modulus - BigUint::from_str("1").unwrap();
    while t.is_even() {
        t = t >> 1;
        s += 1;
    }

    // Compute 2^s root of unity given the generator
    let root_of_unity = biguint_to_u64_vec((exp(generator.clone(), &t, &modulus) * &r) % &modulus, limbs);
    let generator = biguint_to_u64_vec((generator.clone() * &r) % &modulus, limbs);

    let sqrt_impl =
    if (&modulus % BigUint::from_str("4").unwrap()) == BigUint::from_str("3").unwrap() {
        let mod_minus_3_over_4 = biguint_to_u64_vec((&modulus - BigUint::from_str("3").unwrap()) >> 2, limbs);

        // Compute -R as (m - r)
        let rneg = biguint_to_u64_vec(&modulus - &r, limbs);

        quote!{
            impl ::ff::SqrtField for #name {
                fn sqrt(&self) -> Option<Self> {
                    // Shank's algorithm for q mod 4 = 3
                    // https://eprint.iacr.org/2012/685.pdf (page 9, algorithm 2)

                    let mut a1 = self.pow(#mod_minus_3_over_4);

                    let mut a0 = a1;
                    a0.square();
                    a0.mul_assign(self);

                    if a0.0 == #repr(#rneg) {
                        None
                    } else {
                        a1.mul_assign(self);
                        Some(a1)
                    }
                }
            }
        }
    } else if (&modulus % BigUint::from_str("16").unwrap()) == BigUint::from_str("1").unwrap() {
        let mod_minus_1_over_2 = biguint_to_u64_vec((&modulus - BigUint::from_str("1").unwrap()) >> 1, limbs);
        let t_plus_1_over_2 = biguint_to_u64_vec((&t + BigUint::one()) >> 1, limbs);
        let t = biguint_to_u64_vec(t.clone(), limbs);

        quote!{
            impl ::ff::SqrtField for #name {
                fn sqrt(&self) -> Option<Self> {
                    // Tonelli-Shank's algorithm for q mod 16 = 1
                    // https://eprint.iacr.org/2012/685.pdf (page 12, algorithm 5)

                    if self.is_zero() {
                        return Some(*self);
                    }

                    if self.pow(#mod_minus_1_over_2) != Self::one() {
                        None
                    } else {
                        let mut c = #name(#repr(#root_of_unity));
                        let mut r = self.pow(#t_plus_1_over_2);
                        let mut t = self.pow(#t);
                        let mut m = #s;

                        while t != Self::one() {
                            let mut i = 1;
                            {
                                let mut t2i = t;
                                t2i.square();
                                loop {
                                    if t2i == Self::one() {
                                        break;
                                    }
                                    t2i.square();
                                    i += 1;
                                }
                            }

                            for _ in 0..(m - i - 1) {
                                c.square();
                            }
                            r.mul_assign(&c);
                            c.square();
                            t.mul_assign(&c);
                            m = i;
                        }

                        Some(r)
                    }
                }
            }
        }
    } else {
        quote!{}
    };

    // Compute R^2 mod m
    let r2 = biguint_to_u64_vec((&r * &r) % &modulus, limbs);

    let r = biguint_to_u64_vec(r, limbs);
    let modulus = biguint_to_u64_vec(modulus, limbs);

    // Compute -m^-1 mod 2**64 by exponentiating by totient(2**64) - 1
    let mut inv = 1u64;
    for _ in 0..63 {
        inv = inv.wrapping_mul(inv);
        inv = inv.wrapping_mul(modulus[0]);
    }
    inv = inv.wrapping_neg();

    quote! {
        /// This is the modulus m of the prime field
        const MODULUS: #repr = #repr(#modulus);

        /// The number of bits needed to represent the modulus.
        const MODULUS_BITS: u32 = #modulus_num_bits;

        /// The number of bits that must be shaved from the beginning of
        /// the representation when randomly sampling.
        const REPR_SHAVE_BITS: u32 = #repr_shave_bits;

        /// 2^{limbs*64} mod m
        const R: #repr = #repr(#r);

        /// 2^{limbs*64*2} mod m
        const R2: #repr = #repr(#r2);

        /// -(m^{-1} mod m) mod m
        const INV: u64 = #inv;

        /// Multiplicative generator of `MODULUS` - 1 order, also quadratic
        /// nonresidue.
        const GENERATOR: #repr = #repr(#generator);

        /// 2^s * t = MODULUS - 1 with t odd
        const S: usize = #s;

        /// 2^s root of unity computed by GENERATOR^t
        const ROOT_OF_UNITY: #repr = #repr(#root_of_unity);

        #sqrt_impl
    }
}

/// Implement PrimeField for the derived type.
fn prime_field_impl(
    name: &syn::Ident,
    repr: &syn::Ident,
    limbs: usize
) -> quote::Tokens
{
    // Returns r{n} as an ident.
    fn get_temp(n: usize) -> syn::Ident {
        syn::Ident::from(format!("r{}", n))
    }

    // The parameter list for the mont_reduce() internal method.
    // r0: u64, mut r1: u64, mut r2: u64, ...
    let mut mont_paramlist = quote::Tokens::new();
    mont_paramlist.append_separated(
        (0..(limbs*2)).map(|i| (i, get_temp(i)))
               .map(|(i, x)| {
                    if i != 0 {
                        quote!{mut #x: u64}
                    } else {
                        quote!{#x: u64}
                    }
                }),
        ","
    );

    // Implement montgomery reduction for some number of limbs
    fn mont_impl(limbs: usize) -> quote::Tokens
    {
        let mut gen = quote::Tokens::new();

        for i in 0..limbs {
            {
                let temp = get_temp(i);
                gen.append(quote!{
                    let k = #temp.wrapping_mul(INV);
                    let mut carry = 0;
                    ::ff::mac_with_carry(#temp, k, MODULUS.0[0], &mut carry);
                });
            }

            for j in 1..limbs {
                let temp = get_temp(i + j);
                gen.append(quote!{
                    #temp = ::ff::mac_with_carry(#temp, k, MODULUS.0[#j], &mut carry);
                });
            }

            let temp = get_temp(i + limbs);

            if i == 0 {
                gen.append(quote!{
                    #temp = ::ff::adc(#temp, 0, &mut carry);
                });
            } else {
                gen.append(quote!{
                    #temp = ::ff::adc(#temp, carry2, &mut carry);
                });
            }

            if i != (limbs - 1) {
                gen.append(quote!{
                    let carry2 = carry;
                });
            }
        }

        for i in 0..limbs {
            let temp = get_temp(limbs + i);

            gen.append(quote!{
                (self.0).0[#i] = #temp;
            });
        }

        gen
    }

    fn sqr_impl(a: quote::Tokens, limbs: usize) -> quote::Tokens
    {
        let mut gen = quote::Tokens::new();

        for i in 0..(limbs-1) {
            gen.append(quote!{
                let mut carry = 0;
            });

            for j in (i+1)..limbs {
                let temp = get_temp(i + j);
                if i == 0 {
                    gen.append(quote!{
                        let #temp = ::ff::mac_with_carry(0, (#a.0).0[#i], (#a.0).0[#j], &mut carry);
                    });
                } else {
                    gen.append(quote!{
                        let #temp = ::ff::mac_with_carry(#temp, (#a.0).0[#i], (#a.0).0[#j], &mut carry);
                    });
                }
            }

            let temp = get_temp(i + limbs);

            gen.append(quote!{
                let #temp = carry;
            });
        }

        for i in 1..(limbs*2) {
            let k = get_temp(i);

            if i == 1 {
                gen.append(quote!{
                    let tmp0 = #k >> 63;
                    let #k = #k << 1;
                });
            } else if i == (limbs*2 - 1) {
                gen.append(quote!{
                    let #k = tmp0;
                });
            } else {
                gen.append(quote!{
                    let tmp1 = #k >> 63;
                    let #k = #k << 1;
                    let #k = #k | tmp0;
                    let tmp0 = tmp1;
                });
            }
        }

        gen.append(quote!{
            let mut carry = 0;
        });

        for i in 0..limbs {
            let temp0 = get_temp(i * 2);
            let temp1 = get_temp(i * 2 + 1);
            if i == 0 {
                gen.append(quote!{
                    let #temp0 = ::ff::mac_with_carry(0, (#a.0).0[#i], (#a.0).0[#i], &mut carry);
                });
            } else {
                gen.append(quote!{
                    let #temp0 = ::ff::mac_with_carry(#temp0, (#a.0).0[#i], (#a.0).0[#i], &mut carry);
                });
            }

            gen.append(quote!{
                let #temp1 = ::ff::adc(#temp1, 0, &mut carry);
            });
        }

        let mut mont_calling = quote::Tokens::new();
        mont_calling.append_separated((0..(limbs*2)).map(|i| get_temp(i)), ",");

        gen.append(quote!{
            self.mont_reduce(#mont_calling);
        });

        gen
    }

    fn mul_impl(a: quote::Tokens, b: quote::Tokens, limbs: usize) -> quote::Tokens
    {
        let mut gen = quote::Tokens::new();

        for i in 0..limbs {
            gen.append(quote!{
                let mut carry = 0;
            });

            for j in 0..limbs {
                let temp = get_temp(i + j);

                if i == 0 {
                    gen.append(quote!{
                        let #temp = ::ff::mac_with_carry(0, (#a.0).0[#i], (#b.0).0[#j], &mut carry);
                    });
                } else {
                    gen.append(quote!{
                        let #temp = ::ff::mac_with_carry(#temp, (#a.0).0[#i], (#b.0).0[#j], &mut carry);
                    });
                }
            }

            let temp = get_temp(i + limbs);

            gen.append(quote!{
                let #temp = carry;
            });
        }

        let mut mont_calling = quote::Tokens::new();
        mont_calling.append_separated((0..(limbs*2)).map(|i| get_temp(i)), ",");

        gen.append(quote!{
            self.mont_reduce(#mont_calling);
        });

        gen
    }

    let squaring_impl = sqr_impl(quote!{self}, limbs);
    let multiply_impl = mul_impl(quote!{self}, quote!{other}, limbs);
    let montgomery_impl = mont_impl(limbs);

    // (self.0).0[0], (self.0).0[1], ..., 0, 0, 0, 0, ...
    let mut into_repr_params = quote::Tokens::new();
    into_repr_params.append_separated(
        (0..limbs).map(|i| quote!{ (self.0).0[#i] })
                  .chain((0..limbs).map(|_| quote!{0})),
        ","
    );

    quote!{
        impl Copy for #name { }

        impl Clone for #name {
            fn clone(&self) -> #name {
                *self
            }
        }

        impl PartialEq for #name {
            fn eq(&self, other: &#name) -> bool {
                self.0 == other.0
            }
        }

        impl Eq for #name { }

        impl ::std::fmt::Debug for #name
        {
            fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                write!(f, "{}({:?})", stringify!(#name), self.into_repr())
            }
        }

        impl ::rand::Rand for #name {
            /// Computes a uniformly random element using rejection sampling.
            fn rand<R: ::rand::Rng>(rng: &mut R) -> Self {
                loop {
                    let mut tmp = #name(#repr::rand(rng));
                    for _ in 0..REPR_SHAVE_BITS {
                        tmp.0.div2();
                    }
                    if tmp.is_valid() {
                        return tmp
                    }
                }
            }
        }

        impl ::ff::PrimeField for #name {
            type Repr = #repr;

            fn from_repr(r: #repr) -> Result<#name, ()> {
                let mut r = #name(r);
                if r.is_valid() {
                    r.mul_assign(&#name(R2));

                    Ok(r)
                } else {
                    Err(())
                }
            }

            fn into_repr(&self) -> #repr {
                let mut r = *self;
                r.mont_reduce(
                    #into_repr_params
                );

                r.0
            }

            fn char() -> #repr {
                MODULUS
            }

            fn num_bits() -> u32 {
                MODULUS_BITS
            }

            fn capacity() -> u32 {
                Self::num_bits() - 1
            }

            fn multiplicative_generator() -> Self {
                #name(GENERATOR)
            }

            fn s() -> usize {
                S
            }

            fn root_of_unity() -> Self {
                #name(ROOT_OF_UNITY)
            }
        }

        impl ::ff::Field for #name {
            #[inline]
            fn zero() -> Self {
                #name(#repr::from(0))
            }

            #[inline]
            fn one() -> Self {
                #name(R)
            }

            #[inline]
            fn is_zero(&self) -> bool {
                self.0.is_zero()
            }

            #[inline]
            fn add_assign(&mut self, other: &#name) {
                // This cannot exceed the backing capacity.
                self.0.add_nocarry(&other.0);

                // However, it may need to be reduced.
                self.reduce();
            }

            #[inline]
            fn double(&mut self) {
                // This cannot exceed the backing capacity.
                self.0.mul2();

                // However, it may need to be reduced.
                self.reduce();
            }

            #[inline]
            fn sub_assign(&mut self, other: &#name) {
                // If `other` is larger than `self`, we'll need to add the modulus to self first.
                if other.0 > self.0 {
                    self.0.add_nocarry(&MODULUS);
                }

                self.0.sub_noborrow(&other.0);
            }

            #[inline]
            fn negate(&mut self) {
                if !self.is_zero() {
                    let mut tmp = MODULUS;
                    tmp.sub_noborrow(&self.0);
                    self.0 = tmp;
                }
            }

            fn inverse(&self) -> Option<Self> {
                if self.is_zero() {
                    None
                } else {
                    // Guajardo Kumar Paar Pelzl
                    // Efficient Software-Implementation of Finite Fields with Applications to Cryptography
                    // Algorithm 16 (BEA for Inversion in Fp)

                    let one = #repr::from(1);

                    let mut u = self.0;
                    let mut v = MODULUS;
                    let mut b = #name(R2); // Avoids unnecessary reduction step.
                    let mut c = Self::zero();

                    while u != one && v != one {
                        while u.is_even() {
                            u.div2();

                            if b.0.is_even() {
                                b.0.div2();
                            } else {
                                b.0.add_nocarry(&MODULUS);
                                b.0.div2();
                            }
                        }

                        while v.is_even() {
                            v.div2();

                            if c.0.is_even() {
                                c.0.div2();
                            } else {
                                c.0.add_nocarry(&MODULUS);
                                c.0.div2();
                            }
                        }

                        if v < u {
                            u.sub_noborrow(&v);
                            b.sub_assign(&c);
                        } else {
                            v.sub_noborrow(&u);
                            c.sub_assign(&b);
                        }
                    }

                    if u == one {
                        Some(b)
                    } else {
                        Some(c)
                    }
                }
            }

            #[inline(always)]
            fn frobenius_map(&mut self, _: usize) {
                // This has no effect in a prime field.
            }

            #[inline]
            fn mul_assign(&mut self, other: &#name)
            {
                #multiply_impl
            }

            #[inline]
            fn square(&mut self)
            {
                #squaring_impl
            }
        }

        impl #name {
            /// Determines if the element is really in the field. This is only used
            /// internally.
            #[inline(always)]
            fn is_valid(&self) -> bool {
                self.0 < MODULUS
            }

            /// Subtracts the modulus from this element if this element is not in the
            /// field. Only used interally.
            #[inline(always)]
            fn reduce(&mut self) {
                if !self.is_valid() {
                    self.0.sub_noborrow(&MODULUS);
                }
            }

            #[inline(always)]
            fn mont_reduce(
                &mut self,
                #mont_paramlist
            )
            {
                // The Montgomery reduction here is based on Algorithm 14.32 in
                // Handbook of Applied Cryptography
                // <http://cacr.uwaterloo.ca/hac/about/chap14.pdf>.

                #montgomery_impl

                self.reduce();
            }
        }
    }
}
