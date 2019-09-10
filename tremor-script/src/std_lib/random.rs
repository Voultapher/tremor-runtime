// Copyright 2018-2019, Wayfair GmbH
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::registry::{mfa, Context, FResult, FunctionError, Registry, TremorFnWrapper};
use crate::tremor_fn;
use rand::distributions::Alphanumeric;
use rand::Rng;
use simd_json::BorrowedValue as Value;

pub fn load<Ctx: 'static + Context>(registry: &mut Registry<Ctx>) {
    // The random number generator used for these functions is ThreadRng, which
    // is initialized once per thread (and also periodically seeded by the system).
    //
    // TODO see if we can cache it across function calls too.
    // also swap to SmallRng if we can -- should be faster.
    // https://docs.rs/rand/0.7.0/rand/rngs/index.html#our-generators
    //
    // also, `rng.gen_range()` calls here are optimized for a single sample from
    // the range. we will be sampling a lot during a typical use case, so swap it
    // for a distribution based sampling (if we can cache the distribution):
    // https://docs.rs/rand/0.7.0/rand/trait.Rng.html#method.gen_range
    fn integer<'event, Ctx: Context + 'static>(
        _context: &Ctx,
        args: &[&Value<'event>],
    ) -> FResult<Value<'event>> {
        let this_mfa = || mfa("random", "integer", args.len());
        let mut rng = rand::thread_rng();
        match args.len() {
            2 => {
                let (low, high) = (&args[0], &args[1]);
                match (low, high) {
                    (Value::I64(low), Value::I64(high)) if low < high => Ok(Value::I64(
                        // random integer between low and high (not including high)
                        rng.gen_range(low, high),
                    )),
                    (Value::I64(_low), Value::I64(_high)) => Err(FunctionError::RuntimeError {
                        mfa: this_mfa(),
                        error:
                            "Invalid arguments. First argument must be lower than second argument"
                                .to_string(),
                    }),
                    _ => Err(FunctionError::BadType { mfa: this_mfa() }),
                }
            }
            1 => {
                let input = &args[0];
                match input {
                    Value::I64(input) if *input > 0 => Ok(Value::I64(
                        // random integer between 0 and input (not including input)
                        rng.gen_range(0, input),
                    )),
                    Value::I64(_input) => Err(FunctionError::RuntimeError {
                        mfa: this_mfa(),
                        error: "Invalid argument. Must be greater than 0".to_string(),
                    }),
                    _ => Err(FunctionError::BadType { mfa: this_mfa() }),
                }
            }
            0 => Ok(Value::I64(
                rng.gen(), // random integer
            )),
            _ => Err(FunctionError::BadArity {
                mfa: this_mfa(),
                calling_a: args.len(),
            }),
        }
    }
    // TODO try to consolidate this with the integer implementation -- mostly a copy-pasta
    // of that right now, with types changed
    fn float<'event, Ctx: Context + 'static>(
        _context: &Ctx,
        args: &[&Value<'event>],
    ) -> FResult<Value<'event>> {
        let this_mfa = || mfa("random", "float", args.len());
        let mut rng = rand::thread_rng();
        match args.len() {
            2 => {
                let (low, high) = (&args[0], &args[1]);
                match (low, high) {
                    (Value::F64(low), Value::F64(high)) if low < high => Ok(Value::F64(
                        // random float between low and high (not including high)
                        rng.gen_range(low, high),
                    )),
                    (Value::F64(_low), Value::F64(_high)) => Err(FunctionError::RuntimeError {
                        mfa: this_mfa(),
                        error:
                            "Invalid arguments. First argument must be lower than second argument"
                                .to_string(),
                    }),
                    _ => Err(FunctionError::BadType { mfa: this_mfa() }),
                }
            }
            1 => {
                let input = &args[0];
                match input {
                    Value::F64(input) if *input > 0.0 => Ok(Value::F64(
                        // random float between 0 and input (not including input)
                        rng.gen_range(0.0, input),
                    )),
                    Value::F64(_input) => Err(FunctionError::RuntimeError {
                        mfa: this_mfa(),
                        error: "Invalid argument. Must be greater than 0.0".to_string(),
                    }),
                    _ => Err(FunctionError::BadType { mfa: this_mfa() }),
                }
            }
            0 => Ok(Value::F64(
                rng.gen(), // random float (between 0.0 and 1.0, not including 1.0)
            )),
            _ => Err(FunctionError::BadArity {
                mfa: this_mfa(),
                calling_a: args.len(),
            }),
        }
    }
    registry
        .insert(tremor_fn! (random::bool(_context) {
            Ok(Value::Bool(rand::thread_rng().gen()))
        }))
        // TODO support specifying range of characters as a second (optional) arg
        .insert(tremor_fn! (random::string(_context, _length) {
            match _length {
                Value::I64(n) if *n >= 0 => Ok(Value::String(
                    // random string with chars uniformly distributed over ASCII letters and numbers
                    rand::thread_rng().sample_iter(&Alphanumeric).take(*n as usize).collect()
                )),
                Value::I64(_) => Err(FunctionError::RuntimeError {
                    mfa: this_mfa(),
                    error: "Invalid argument. Must be greater than or equal to 0".to_string(),
                }),
                _ => Err(FunctionError::BadType{mfa: this_mfa()}),
            }
        }))
        .insert(TremorFnWrapper {
            module: "random".to_string(),
            name: "integer".to_string(),
            fun: integer,
            argc: 0,
        })
        .insert(TremorFnWrapper {
            module: "random".to_string(),
            name: "float".to_string(),
            fun: float,
            argc: 0,
        });
}

#[cfg(test)]
mod test {
    use crate::registry::fun;
    use simd_json::BorrowedValue as Value;

    macro_rules! assert_val {
        ($e:expr, $r:expr) => {
            assert_eq!($e, Ok(Value::from($r)))
        };
    }

    #[test]
    fn bool() {
        let f = fun("random", "bool");
        assert!(match f(&[]) {
            Ok(Value::Bool(_)) => true,
            _ => false,
        });
    }

    #[test]
    fn string() {
        let f = fun("random", "string");
        let n = 0;
        assert_val!(f(&[&Value::from(n)]), "");
        let n = 16;
        assert!(match f(&[&Value::from(n)]) {
            Ok(Value::String(s)) => s.len() as i64 == n,
            _ => false,
        });
    }

    #[test]
    fn integer() {
        let f = fun("random", "integer");
        let v1 = Value::from(0);
        let v2 = Value::from(1);
        assert_val!(f(&[&v1, &v2]), 0);
        let v1 = Value::from(-42);
        let v2 = Value::from(-41);
        assert_val!(f(&[&v1, &v2]), -42);
        let v = Value::from(1);
        assert_val!(f(&[&v]), 0);
        assert!(match f(&[]) {
            Ok(Value::I64(_)) => true,
            _ => false,
        });
    }

    #[test]
    fn float() {
        let f = fun("random", "float");
        let v1 = 0.0;
        let v2 = 100.0;
        assert!(match f(&[&Value::from(v1), &Value::from(v2)]) {
            Ok(Value::F64(a)) if a >= v1 && a < v2 => true,
            _ => false,
        });
        let v = 100.0;
        assert!(match f(&[&Value::from(v)]) {
            Ok(Value::F64(a)) if a >= 0.0 && a < v => true,
            _ => false,
        });
        assert!(match f(&[]) {
            Ok(Value::F64(a)) if a >= 0.0 && a < 1.0 => true,
            _ => false,
        });
    }
}