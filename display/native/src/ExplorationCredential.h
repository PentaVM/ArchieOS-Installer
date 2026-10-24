#pragma once
#include <QByteArray>
#include <QString>
// Read-only World projection credential. Never reads the command descriptor.
QString explorationRuntimeDirectory();
QByteArray explorationProjectionToken(const QString &runtimeDirectory);
