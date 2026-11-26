pub use crate::avm2::object::dictionary_allocator;

use crate::avm2::{activation::Activation, function::FunctionArgs, Error, Value};

pub fn enable_weak_keys<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    _args: FunctionArgs<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    let dictionary = this.as_object().unwrap().as_dictionary_object().unwrap();
    dictionary.enable_weak_keys(activation);
    Ok(Value::Undefined)
}

pub fn aoqi_use_display_owners<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    _args: FunctionArgs<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    this.as_object().unwrap().as_dictionary_object().unwrap().use_display_owners(activation.gc());
    Ok(this)
}
