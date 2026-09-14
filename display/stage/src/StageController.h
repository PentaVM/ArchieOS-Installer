#pragma once

// The compositor's controller: it owns the stage model, listens for sealed commands,
// delivers the agent's input to applications, watches physical input for a person taking
// over, and publishes what the director decides to draw.
//
// Nothing here decides anything the pure model in `display/visual` does not already
// decide. This file routes: bytes from a socket into `decode_command`, a decoded command
// into `check_command` and `Playback`, the steps a playback emits into `QWaylandSeat`, and
// each `StageFrame` into properties a QML scene binds to.

#include <QElapsedTimer>
#include <QHash>
#include <QList>
#include <QLocalServer>
#include <QLocalSocket>
#include <QObject>
#include <QPointer>
#include <QQuickWindow>
#include <QTimer>
#include <QVariantList>
#include <QWaylandCompositor>
#include <QWaylandQuickItem>
#include <QWaylandSeat>

#include <memory>
#include <optional>

#include "CommandCodec.h"
#include "KeyTyper.h"
#include "archie/visual/director.hpp"
#include "archie/visual/stage.hpp"

class QWaylandSurface;

class StageController final : public QObject
{
    Q_OBJECT
    Q_PROPERTY(QWaylandCompositor *compositor READ compositor WRITE setCompositor NOTIFY compositorChanged)
    Q_PROPERTY(QQuickWindow *window READ window WRITE setWindow NOTIFY windowChanged)
    Q_PROPERTY(QObject *desk READ desk WRITE setDesk NOTIFY deskChanged)

    // What the director decided, in screen coordinates.
    Q_PROPERTY(qreal viewX READ viewX NOTIFY frameChanged)
    Q_PROPERTY(qreal viewY READ viewY NOTIFY frameChanged)
    Q_PROPERTY(qreal viewZoom READ viewZoom NOTIFY frameChanged)
    Q_PROPERTY(bool cursorVisible READ cursorVisible NOTIFY frameChanged)
    Q_PROPERTY(qreal cursorX READ cursorX NOTIFY frameChanged)
    Q_PROPERTY(qreal cursorY READ cursorY NOTIFY frameChanged)
    Q_PROPERTY(qreal cursorOpacity READ cursorOpacity NOTIFY frameChanged)
    Q_PROPERTY(bool cursorPressed READ cursorPressed NOTIFY frameChanged)
    Q_PROPERTY(bool dragging READ dragging NOTIFY frameChanged)
    Q_PROPERTY(qreal dragFromX READ dragFromX NOTIFY frameChanged)
    Q_PROPERTY(qreal dragFromY READ dragFromY NOTIFY frameChanged)
    Q_PROPERTY(QVariantList ripples READ ripples NOTIFY frameChanged)
    Q_PROPERTY(QString captionVerb READ captionVerb NOTIFY frameChanged)
    Q_PROPERTY(QString captionApp READ captionApp NOTIFY frameChanged)
    Q_PROPERTY(int captionCount READ captionCount NOTIFY frameChanged)
    Q_PROPERTY(QString captionRefusal READ captionRefusal NOTIFY frameChanged)
    Q_PROPERTY(qreal captionOpacity READ captionOpacity NOTIFY frameChanged)
    Q_PROPERTY(QVariantList activity READ activity NOTIFY frameChanged)
    Q_PROPERTY(QString banner READ banner NOTIFY frameChanged)
    Q_PROPERTY(qreal bannerOpacity READ bannerOpacity NOTIFY frameChanged)
    Q_PROPERTY(QString authority READ authority NOTIFY authorityChanged)
    // Whether a person's mouse and keyboard reach applications. Surfaces take input from the
    // scene only while this holds: otherwise Qt Quick would forward hover it synthesises
    // under a still physical pointer, and move the agent's pointer out from under a click.
    Q_PROPERTY(bool physicalInput READ physicalInput NOTIFY authorityChanged)
    Q_PROPERTY(int surfaceCount READ surfaceCount NOTIFY surfacesChanged)
    Q_PROPERTY(QObject *focusedItem READ focusedItem NOTIFY focusChanged)
    Q_PROPERTY(QString controlSocket READ controlSocket CONSTANT)
    Q_PROPERTY(bool listening READ listening NOTIFY listeningChanged)

public:
    struct Options {
        QString control_socket;
        QByteArray key;
        bool reduced_motion = false;
        archie::visual::RenderTier tier = archie::visual::RenderTier::Normal;
    };

    explicit StageController(Options options, QObject *parent = nullptr);
    ~StageController() override;

    QWaylandCompositor *compositor() const { return m_compositor; }
    void setCompositor(QWaylandCompositor *compositor);
    QQuickWindow *window() const { return m_window; }
    void setWindow(QQuickWindow *window);
    QObject *desk() const { return m_desk; }
    void setDesk(QObject *desk);

    qreal viewX() const { return m_viewX; }
    qreal viewY() const { return m_viewY; }
    qreal viewZoom() const { return m_viewZoom; }
    bool cursorVisible() const { return m_frame.cursor_visible; }
    qreal cursorX() const { return m_cursorScreenX; }
    qreal cursorY() const { return m_cursorScreenY; }
    qreal cursorOpacity() const { return m_frame.cursor_opacity; }
    bool cursorPressed() const { return m_frame.cursor_pressed; }
    bool dragging() const { return m_frame.drag_from.has_value(); }
    qreal dragFromX() const { return m_dragScreenX; }
    qreal dragFromY() const { return m_dragScreenY; }
    QVariantList ripples() const { return m_ripples; }
    QString captionVerb() const { return m_captionVerb; }
    QString captionApp() const { return m_captionApp; }
    int captionCount() const { return m_captionCount; }
    QString captionRefusal() const { return m_captionRefusal; }
    qreal captionOpacity() const { return m_captionOpacity; }
    QVariantList activity() const { return m_activity; }
    QString banner() const { return m_banner; }
    qreal bannerOpacity() const { return m_frame.banner_opacity; }
    QString authority() const;
    bool physicalInput() const { return m_authority.forwards_physical_input(); }
    int surfaceCount() const { return static_cast<int>(m_items.size()); }
    QObject *focusedItem() const { return m_focusedItem; }
    QString controlSocket() const { return m_options.control_socket; }
    bool listening() const { return m_server.isListening(); }

    /// A toplevel became visible. Called from QML once the item exists.
    Q_INVOKABLE void registerSurface(QWaylandQuickItem *item, const QString &appId);
    /// The item's desk geometry or application id may have changed.
    Q_INVOKABLE void updateSurface(QWaylandQuickItem *item, const QString &appId);
    Q_INVOKABLE void unregisterSurface(QWaylandQuickItem *item);
    Q_INVOKABLE void setOutputSize(int width, int height);

signals:
    void compositorChanged();
    void windowChanged();
    void deskChanged();
    void frameChanged();
    void authorityChanged();
    void surfacesChanged();
    void focusChanged();
    void listeningChanged();

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    std::int64_t monotonic() const { return m_clock.elapsed(); }
    static std::int64_t wallclock();

    void acceptConnection();
    void readFrom(QLocalSocket *socket);
    void handleLine(QLocalSocket *socket, const QByteArray &line);
    void greet(QLocalSocket *socket);
    void broadcast(const QByteArray &line);
    QByteArray receipt(std::uint64_t sequence, const QString &outcome) const;
    void sendReceipt(std::uint64_t sequence, const QString &outcome);

    std::optional<archie::visual::CommandRefusal> undeliverable(const archie::stage::DecodedCommand &command);
    void deliver(const archie::visual::InputStep &step);
    void cancelPlayback(const char *outcome);
    void failPlayback(archie::visual::CommandRefusal refusal);
    void stopPlayback(const QString &outcome, std::optional<archie::visual::CommandRefusal> failure);
    void watchSeat();
    void handleKeyboardFocus(QWaylandSurface *newFocus);
    KeyTyper *typer();
    void noteAuthority(archie::visual::AuthorityMode before);
    void tick();
    void publish(const archie::visual::StageFrame &frame);
    void schedule(std::optional<std::int64_t> at);

    QByteArray surfaceEvent(archie::visual::SurfaceId id) const;
    QWaylandSeat *seat() const;
    QWaylandQuickItem *itemFor(archie::visual::SurfaceId id) const;

    Options m_options;
    QPointer<QWaylandCompositor> m_compositor;
    QPointer<QWaylandSeat> m_watchedSeat;
    std::unique_ptr<KeyTyper> m_typer;
    QPointer<QQuickWindow> m_window;
    QPointer<QObject> m_desk;

    QLocalServer m_server;
    QList<QPointer<QLocalSocket>> m_clients;
    QHash<QLocalSocket *, QByteArray> m_buffers;

    QElapsedTimer m_clock;
    QTimer m_timer;

    archie::visual::SurfaceTable m_table;
    archie::visual::InputAuthority m_authority;
    archie::visual::StageDirector m_director;
    archie::visual::PlaybackPolicy m_playbackPolicy;
    std::optional<archie::visual::Playback> m_playback;
    std::optional<archie::stage::DecodedCommand> m_active;
    /// The route the active text takes, chosen at its first character.
    std::optional<KeyTyper::Route> m_textRoute;
    archie::visual::PointF m_pointer{};
    std::uint64_t m_lastSequence = 0;
    archie::visual::SurfaceId m_nextSurface = 1;
    QHash<archie::visual::SurfaceId, QPointer<QWaylandQuickItem>> m_items;
    QHash<QWaylandQuickItem *, archie::visual::SurfaceId> m_ids;
    QPointer<QObject> m_focusedItem;

    QPointF m_lastPhysicalPointer;
    bool m_havePhysicalPointer = false;
    /// Physical keys down right now. The compositor sends their modifier state to the
    /// focused application whatever this controller does with the key events, so the agent
    /// does not act while a person's hand is still on the keyboard.
    int m_physicalKeysHeld = 0;

    archie::visual::StageFrame m_frame{};
    qreal m_viewX = 0;
    qreal m_viewY = 0;
    qreal m_viewZoom = 1;
    qreal m_cursorScreenX = 0;
    qreal m_cursorScreenY = 0;
    qreal m_dragScreenX = 0;
    qreal m_dragScreenY = 0;
    QVariantList m_ripples;
    QVariantList m_activity;
    QString m_captionVerb;
    QString m_captionApp;
    int m_captionCount = 0;
    QString m_captionRefusal;
    qreal m_captionOpacity = 0;
    QString m_banner;
    int m_outputWidth = 1920;
    int m_outputHeight = 1080;
};
