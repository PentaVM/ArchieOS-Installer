#include "Payload.h"

#include <QJsonArray>
#include <QJsonObject>

#include <string>
#include <vector>

namespace archie::native {

using archie::visual::Member;
using archie::visual::Value;

Value fromJson(const QJsonValue &value)
{
    switch (value.type()) {
    case QJsonValue::Bool:
        return Value(value.toBool());
    case QJsonValue::Double:
        return Value(value.toDouble());
    case QJsonValue::String:
        return Value(value.toString().toStdString());
    case QJsonValue::Array: {
        const auto source = value.toArray();
        std::vector<Value> items;
        items.reserve(static_cast<std::size_t>(source.size()));
        for (const auto &item : source)
            items.push_back(fromJson(item));
        return Value::array(std::move(items));
    }
    case QJsonValue::Object: {
        const auto source = value.toObject();
        std::vector<Member> members;
        members.reserve(static_cast<std::size_t>(source.size()));
        // Qt sorts object keys. The model keeps insertion order so a duplicate key stays
        // visible rather than being collapsed by a map — Qt has already collapsed it, so
        // what arrives here is what the transport was able to represent, and the model's
        // ordering guarantee holds for everything downstream of this point.
        for (auto it = source.constBegin(); it != source.constEnd(); ++it)
            members.push_back(Member{it.key().toStdString(), fromJson(it.value())});
        return Value::object(std::move(members));
    }
    case QJsonValue::Null:
    case QJsonValue::Undefined:
        break;
    }
    return Value{};
}

}  // namespace archie::native
