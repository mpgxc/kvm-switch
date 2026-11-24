// Troca pela borda no GNOME: no Wayland um app não lê a posição global do cursor,
// então esta extensão põe barreiras de pressão (as mesmas dos cantos ativos) nas
// bordas externas da área de trabalho. Empurrar uma delas roda
// `kvm-switch edge <lado>`; o kvm-switch lê a configuração e decide se o lado tem
// vizinho, por isso a extensão não sabe nada da configuração.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Layout from 'resource:///org/gnome/shell/ui/layout.js';
import {PACKAGE_VERSION} from 'resource:///org/gnome/shell/misc/config.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

// Os mesmos valores dos cantos ativos do GNOME (layout.js): empurrar ~100 px em 1 s.
const PRESSAO = 100;
const JANELA_MS = 1000;
// Cantos não disparam, para não brigar com o canto ativo nem com o de outra borda.
const MARGEM = 8;
// Espera depois de uma troca antes de aceitar outra.
const PAUSA_MS = 1000;

// Para cada lado: a direção em que a barreira deixa o cursor passar (a de entrar na tela).
const DIRECOES = {
    esquerda: Meta.BarrierDirection.POSITIVE_X,
    direita: Meta.BarrierDirection.NEGATIVE_X,
    cima: Meta.BarrierDirection.POSITIVE_Y,
    baixo: Meta.BarrierDirection.NEGATIVE_Y,
};

/**
 * Trechos das bordas externas da união dos monitores, sem `margem` px em cada ponta.
 * A borda de um monitor encostada em outro não é externa: passar por ela só leva o
 * cursor ao outro monitor deste PC.
 */
export function bordasExternas(monitores, margem) {
    const bordas = [];
    for (const m of monitores) {
        const direita = m.x + m.width;
        const baixo = m.y + m.height;
        const lados = [
            {lado: 'esquerda', vertical: true, fixo: m.x, de: m.y, ate: baixo, encosta: n => n.x + n.width === m.x},
            {lado: 'direita', vertical: true, fixo: direita, de: m.y, ate: baixo, encosta: n => n.x === direita},
            {lado: 'cima', vertical: false, fixo: m.y, de: m.x, ate: direita, encosta: n => n.y + n.height === m.y},
            {lado: 'baixo', vertical: false, fixo: baixo, de: m.x, ate: direita, encosta: n => n.y === baixo},
        ];
        for (const {lado, vertical, fixo, de, ate, encosta} of lados) {
            let trechos = [[de, ate]];
            for (const n of monitores.filter(encosta)) {
                const [a, b] = vertical ? [n.y, n.y + n.height] : [n.x, n.x + n.width];
                trechos = trechos
                    .flatMap(([inicio, fim]) => [[inicio, Math.min(fim, a)], [Math.max(inicio, b), fim]])
                    .filter(([inicio, fim]) => inicio < fim);
            }
            for (const [inicio, fim] of trechos) {
                if (fim - inicio > 2 * margem)
                    bordas.push({lado, vertical, fixo, de: inicio + margem, ate: fim - margem});
            }
        }
    }
    return bordas;
}

/** O kvm-switch: o PATH do GNOME Shell nem sempre tem o ~/.local/bin do install.sh. */
function programa() {
    return GLib.find_program_in_path('kvm-switch') ??
        GLib.build_filenamev([GLib.get_home_dir(), '.local', 'bin', 'kvm-switch']);
}

export default class KvmSwitchBordas extends Extension {
    enable() {
        this._ocupado = false;
        this._pausa = 0;
        this._barreiras = [];
        this._pressoes = [];
        this._monitores = Main.layoutManager.connect('monitors-changed', () => this._montar());
        this._montar();
    }

    disable() {
        Main.layoutManager.disconnect(this._monitores);
        this._desmontar();
        if (this._pausa)
            GLib.source_remove(this._pausa);
        this._pausa = 0;
        this._ocupado = false;
    }

    /** Barreiras nas bordas externas dos monitores de agora. */
    _montar() {
        this._desmontar();
        // O Mutter 46 trocou `display` por `backend` na barreira.
        const onde = parseInt(PACKAGE_VERSION) >= 46 ? {backend: global.backend} : {display: global.display};
        const porLado = {};
        for (const borda of bordasExternas(Main.layoutManager.monitors, MARGEM)) {
            const [x1, y1, x2, y2] = borda.vertical
                ? [borda.fixo, borda.de, borda.fixo, borda.ate]
                : [borda.de, borda.fixo, borda.ate, borda.fixo];
            const barreira = new Meta.Barrier({...onde, x1, y1, x2, y2, directions: DIRECOES[borda.lado]});
            this._barreiras.push(barreira);
            // O sinal `trigger` não diz qual barreira: uma PressureBarrier por lado.
            if (!porLado[borda.lado]) {
                // Na visão geral também, como os cantos ativos: o GNOME abre nela ao entrar.
                const modos = Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW;
                const pressao = new Layout.PressureBarrier(PRESSAO, JANELA_MS, modos);
                pressao.connect('trigger', () => this._atravessar(borda.lado));
                porLado[borda.lado] = pressao;
                this._pressoes.push(pressao);
            }
            porLado[borda.lado].addBarrier(barreira);
        }
    }

    _desmontar() {
        for (const pressao of this._pressoes)
            pressao.destroy();
        for (const barreira of this._barreiras)
            barreira.destroy();
        this._pressoes = [];
        this._barreiras = [];
    }

    /** Roda `kvm-switch edge <lado>` sem travar o Shell; um por vez, com pausa depois. */
    _atravessar(lado) {
        if (this._ocupado)
            return;
        this._ocupado = true;
        const liberar = () => {
            // Desligada no meio da troca: o disable já soltou tudo.
            if (!this._ocupado)
                return;
            this._pausa = GLib.timeout_add(GLib.PRIORITY_DEFAULT, PAUSA_MS, () => {
                this._pausa = 0;
                this._ocupado = false;
                return GLib.SOURCE_REMOVE;
            });
        };
        let processo;
        try {
            processo = Gio.Subprocess.new([programa(), 'edge', lado], Gio.SubprocessFlags.NONE);
        } catch (erro) {
            console.error(`kvm-switch: não consegui rodar o kvm-switch: ${erro.message}`);
            liberar();
            return;
        }
        processo.wait_async(null, (_processo, resultado) => {
            try {
                processo.wait_finish(resultado);
            } catch (erro) {
                console.error(`kvm-switch: ${erro.message}`);
            }
            liberar();
        });
    }
}
